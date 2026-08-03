//! Download-to-disk store for DRM-free ebook acquisitions.
//!
//! When a user downloads a book from a DOWNLOAD-capable connector (today: an
//! OPDS acquisition link), Libro persists the EPUB into a **managed downloads
//! directory** under the platform data dir and records a small JSON manifest.
//!
//! ## Why this reuses the Local Files plumbing
//! Downloads land as ordinary `.epub` files in a directory that the
//! [`crate::providers::localfiles`] machinery already knows how to scan. That is
//! deliberate: a downloaded book gets the **same** FNV-1a path-hash
//! [`Book::id`](crate::models::Book::id) that
//! [`crate::providers::localfiles::book_id_for_path`] would assign it, so the
//! in-app reader (`get_book_file`), covers (`get_local_cover`) and
//! Send-to-Kindle (`send_to_kindle`) all work on downloaded books with **zero
//! special-casing** — they already operate on locally-scanned EPUBs by
//! path-hash id. The only wiring needed is to include the downloads directory in
//! the id-resolution search path (see the Tauri command layer).
//!
//! ## Legal posture
//! Only the user's **own, DRM-free** acquisitions from configured providers are
//! downloaded. No scraping, no DRM circumvention, no bundled sources. Bytes are
//! content-sniffed as a real EPUB (see
//! [`crate::providers::localfiles::is_epub_bytes`]) before they are persisted, so
//! an HTML error page never lands on disk masquerading as a book.
//!
//! The fetch itself is behind the [`BookFetcher`] seam so the persist / dedup /
//! validate logic is unit-testable with a fake fetcher and **never** touches the
//! network in the test suite — mirroring the [`crate::kindle`] sender seam.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::ConfigError;
use crate::models::{Book, MediaType};
use crate::providers::localfiles::{book_id_for_path, is_epub_bytes};

/// Identifier key under which OPDS stores an item's acquisition URL.
pub const ACQUISITION_URL_KEY: &str = "opds:acquisition_url";

/// Per-file size cap. A downloaded EPUB larger than this is rejected as
/// [`DownloadOutcome::TooLarge`] **before** anything is written to disk.
pub const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024; // 100 MB

/// The downloads directory under the Libro platform data dir
/// (`%APPDATA%/Libro/downloads`, `$XDG_CONFIG_HOME/Libro/downloads`, …).
pub fn downloads_dir() -> PathBuf {
    crate::config::data_dir().join("downloads")
}

/// One record in the downloads manifest.
///
/// `book_id` is the path-hash the reader/cover/kindle commands resolve against —
/// identical to what [`book_id_for_path`] assigns the persisted file — so a
/// downloaded book is addressable exactly like any locally-scanned EPUB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadEntry {
    /// FNV-1a path-hash id of the persisted file (see module docs).
    pub book_id: String,
    pub title: String,
    /// The connector the book was downloaded from (e.g. `"opds"`).
    pub source_provider_id: String,
    /// The acquisition URL the bytes were fetched from; the dedup key.
    pub acquisition_url: String,
    pub media_type: MediaType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    /// Unix epoch seconds at which the download completed.
    pub downloaded_at: u64,
    /// On-disk size in bytes.
    pub size: usize,
    /// The file name (not a full path) under [`downloads_dir`].
    pub filename: String,
}

/// The typed outcome of a download attempt, returned to the frontend.
///
/// Serialized with a `status` tag so the UI can branch (`downloaded`,
/// `already_downloaded`, `not_downloadable`, `too_large`, `not_an_epub`,
/// `failed`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DownloadOutcome {
    /// Freshly fetched, validated and persisted.
    Downloaded { book_id: String, filename: String },
    /// This acquisition URL was already downloaded — nothing refetched.
    AlreadyDownloaded { book_id: String },
    /// The book carries no acquisition URL, so it can't be downloaded.
    NotDownloadable,
    /// The fetched bytes exceed [`MAX_DOWNLOAD_BYTES`]; nothing persisted.
    TooLarge { size: usize, limit: usize },
    /// The fetched bytes are not a real EPUB; nothing persisted.
    NotAnEpub,
    /// The fetch or the disk write failed.
    Failed { reason: String },
}

/// The acquisition URL for a book, if it advertises one.
pub fn acquisition_url(book: &Book) -> Option<&str> {
    book.identifiers
        .get(ACQUISITION_URL_KEY)
        .map(|s| s.as_str())
        .filter(|s| !s.trim().is_empty())
}

/// The fetch seam. The real implementation performs an HTTP GET (see
/// [`ReqwestFetcher`]); tests inject a fake that returns fixture EPUB bytes so
/// the persist/dedup/validate logic runs without a network.
#[async_trait]
pub trait BookFetcher: Send + Sync {
    /// Fetch the raw bytes at `url`, or a human-readable error.
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// A plain HTTP GET fetcher (rustls, no auth). Public OPDS feeds
/// (Project Gutenberg, Standard Ebooks, …) serve acquisition links directly, so
/// this is a sensible default; the Tauri layer may substitute an auth-aware
/// provider-backed fetcher for private feeds.
#[derive(Default)]
pub struct ReqwestFetcher;

#[async_trait]
impl BookFetcher for ReqwestFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        Ok(bytes.to_vec())
    }
}

/// A file-backed store over the managed downloads directory + its JSON manifest.
pub struct DownloadStore {
    dir: PathBuf,
}

impl DownloadStore {
    /// Create a store rooted at an explicit directory (used by tests and by the
    /// Tauri layer, which passes the platform downloads path).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The default store at [`downloads_dir`].
    pub fn default_store() -> Self {
        Self::new(downloads_dir())
    }

    /// The downloads directory (used to extend the reader's id-resolution path).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }

    /// Read the manifest, treating a missing/corrupt file as empty so a bad
    /// write never bricks the library.
    pub fn list(&self) -> Result<Vec<DownloadEntry>, ConfigError> {
        match fs::read(self.manifest_path()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(ConfigError::Io(e.to_string())),
        }
    }

    /// The existing entry for an acquisition URL, if already downloaded.
    pub fn find_by_url(&self, url: &str) -> Result<Option<DownloadEntry>, ConfigError> {
        Ok(self
            .list()?
            .into_iter()
            .find(|e| e.acquisition_url == url))
    }

    /// Whether an acquisition URL has already been downloaded.
    pub fn is_downloaded(&self, url: &str) -> bool {
        self.find_by_url(url).map(|o| o.is_some()).unwrap_or(false)
    }

    fn save_manifest(&self, entries: &[DownloadEntry]) -> Result<(), ConfigError> {
        fs::create_dir_all(&self.dir).map_err(|e| ConfigError::Io(e.to_string()))?;
        let bytes = serde_json::to_vec_pretty(entries)?;
        let path = self.manifest_path();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes).map_err(|e| ConfigError::Io(e.to_string()))?;
        fs::rename(&tmp, &path).map_err(|e| ConfigError::Io(e.to_string()))?;
        // TODO(security): encrypt the manifest at rest with the keychain-held key,
        // mirroring crate::config, once the crypto boundary is wired.
        Ok(())
    }

    /// Persist validated EPUB `bytes` for `book` fetched from `url`: write the
    /// file atomically (temp + rename), compute its reader id, and record the
    /// manifest entry. Assumes the caller has already validated size + EPUB-ness
    /// and checked dedup.
    pub fn persist(&self, book: &Book, url: &str, bytes: &[u8]) -> Result<DownloadEntry, ConfigError> {
        fs::create_dir_all(&self.dir).map_err(|e| ConfigError::Io(e.to_string()))?;

        let filename = safe_filename(&book.title, url);
        let final_path = self.dir.join(&filename);
        let tmp = final_path.with_extension("epub.tmp");
        fs::write(&tmp, bytes).map_err(|e| ConfigError::Io(e.to_string()))?;
        fs::rename(&tmp, &final_path).map_err(|e| ConfigError::Io(e.to_string()))?;

        let entry = DownloadEntry {
            book_id: book_id_for_path(&final_path),
            title: book.title.clone(),
            source_provider_id: book.source_provider_id.clone(),
            acquisition_url: url.to_string(),
            media_type: book.media_type,
            isbn: book.identifiers.get("isbn").cloned(),
            downloaded_at: now_unix(),
            size: bytes.len(),
            filename,
        };

        let mut entries = self.list()?;
        entries.push(entry.clone());
        self.save_manifest(&entries)?;
        Ok(entry)
    }
}

/// Orchestrate a download: resolve the acquisition URL → dedup → fetch → size
/// guard → EPUB sniff → persist + record. Pure of I/O except the injected
/// `fetcher.fetch` and the store writes, so it is fully unit-testable with a
/// fake fetcher. Returns a typed [`DownloadOutcome`] — never panics.
pub async fn download_book_to_store(
    store: &DownloadStore,
    book: &Book,
    fetcher: &dyn BookFetcher,
) -> DownloadOutcome {
    let url = match acquisition_url(book) {
        Some(u) => u.to_string(),
        None => return DownloadOutcome::NotDownloadable,
    };

    match store.find_by_url(&url) {
        Ok(Some(existing)) => {
            return DownloadOutcome::AlreadyDownloaded {
                book_id: existing.book_id,
            }
        }
        Ok(None) => {}
        Err(e) => return DownloadOutcome::Failed { reason: e.to_string() },
    }

    let bytes = match fetcher.fetch(&url).await {
        Ok(b) => b,
        Err(reason) => return DownloadOutcome::Failed { reason },
    };

    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return DownloadOutcome::TooLarge {
            size: bytes.len(),
            limit: MAX_DOWNLOAD_BYTES,
        };
    }

    if !is_epub_bytes(&bytes) {
        return DownloadOutcome::NotAnEpub;
    }

    match store.persist(book, &url, &bytes) {
        Ok(entry) => DownloadOutcome::Downloaded {
            book_id: entry.book_id,
            filename: entry.filename,
        },
        Err(e) => DownloadOutcome::Failed { reason: e.to_string() },
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build a filesystem-safe `.epub` file name from the title, disambiguated by a
/// short hash of the acquisition URL so distinct downloads never collide.
fn safe_filename(title: &str, url: &str) -> String {
    let mut stem: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    stem = stem.trim().replace(' ', "_");
    stem.truncate(60);
    if stem.is_empty() {
        stem = "book".to_string();
    }
    format!("{stem}-{}.epub", short_hash(url))
}

/// A short hex tag (FNV-1a) of `s`, used only to disambiguate file names.
fn short_hash(s: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;
    use zip::write::SimpleFileOptions;

    const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const OPF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Downloaded Title</dc:title>
    <dc:creator>Some Author</dc:creator>
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="content"/></spine>
</package>"#;

    /// Produce a real (in-memory) EPUB zip, mirroring the LocalFiles test EPUB.
    fn epub_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let stored =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("mimetype", stored).unwrap();
            zip.write_all(b"application/epub+zip").unwrap();
            zip.start_file("META-INF/container.xml", stored).unwrap();
            zip.write_all(CONTAINER_XML.as_bytes()).unwrap();
            zip.start_file("OEBPS/content.opf", stored).unwrap();
            zip.write_all(OPF.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf
    }

    fn opds_book(url: Option<&str>) -> Book {
        let mut book = Book::new("opds:1", "Downloaded Title", MediaType::Ebook, "opds");
        if let Some(u) = url {
            book.identifiers
                .insert(ACQUISITION_URL_KEY.to_string(), u.to_string());
        }
        book
    }

    /// A fake fetcher returning canned bytes (or an error), recording call count.
    struct FakeFetcher {
        bytes: Vec<u8>,
        fail_with: Option<String>,
        calls: Mutex<usize>,
    }

    impl FakeFetcher {
        fn ok(bytes: Vec<u8>) -> Self {
            Self { bytes, fail_with: None, calls: Mutex::new(0) }
        }
        fn failing(reason: &str) -> Self {
            Self { bytes: Vec::new(), fail_with: Some(reason.into()), calls: Mutex::new(0) }
        }
    }

    #[async_trait]
    impl BookFetcher for FakeFetcher {
        async fn fetch(&self, _url: &str) -> Result<Vec<u8>, String> {
            *self.calls.lock().unwrap() += 1;
            match &self.fail_with {
                Some(e) => Err(e.clone()),
                None => Ok(self.bytes.clone()),
            }
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn temp_store() -> (tempfile::TempDir, DownloadStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = DownloadStore::new(dir.path().join("downloads"));
        (dir, store)
    }

    #[test]
    fn manifest_round_trips_and_reports_downloaded() {
        let (_g, store) = temp_store();
        let book = opds_book(Some("https://ex.org/a.epub"));
        let entry = store.persist(&book, "https://ex.org/a.epub", &epub_bytes()).unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], entry);
        assert!(store.is_downloaded("https://ex.org/a.epub"));
        assert!(!store.is_downloaded("https://ex.org/other.epub"));
    }

    #[test]
    fn persisted_id_matches_reader_path_hash() {
        let (_g, store) = temp_store();
        let book = opds_book(Some("https://ex.org/a.epub"));
        let entry = store.persist(&book, "https://ex.org/a.epub", &epub_bytes()).unwrap();

        // The reader recomputes book_id_for_path over the same file → must match.
        let on_disk = store.dir().join(&entry.filename);
        assert_eq!(entry.book_id, book_id_for_path(&on_disk));
    }

    #[test]
    fn happy_path_downloads_and_records_once() {
        let (_g, store) = temp_store();
        let book = opds_book(Some("https://ex.org/a.epub"));
        let fetcher = FakeFetcher::ok(epub_bytes());

        let outcome = block_on(download_book_to_store(&store, &book, &fetcher));
        match outcome {
            DownloadOutcome::Downloaded { book_id, filename } => {
                assert!(filename.ends_with(".epub"));
                assert!(!book_id.is_empty());
            }
            other => panic!("expected Downloaded, got {other:?}"),
        }
        assert_eq!(*fetcher.calls.lock().unwrap(), 1);
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn second_identical_download_is_idempotent_no_refetch() {
        let (_g, store) = temp_store();
        let book = opds_book(Some("https://ex.org/a.epub"));

        let first = FakeFetcher::ok(epub_bytes());
        assert!(matches!(
            block_on(download_book_to_store(&store, &book, &first)),
            DownloadOutcome::Downloaded { .. }
        ));

        let second = FakeFetcher::ok(epub_bytes());
        let outcome = block_on(download_book_to_store(&store, &book, &second));
        assert!(matches!(outcome, DownloadOutcome::AlreadyDownloaded { .. }));
        // Dedup happens before fetch: the second fetcher is never called.
        assert_eq!(*second.calls.lock().unwrap(), 0);
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn missing_acquisition_url_is_not_downloadable() {
        let (_g, store) = temp_store();
        let book = opds_book(None);
        let fetcher = FakeFetcher::ok(epub_bytes());
        let outcome = block_on(download_book_to_store(&store, &book, &fetcher));
        assert_eq!(outcome, DownloadOutcome::NotDownloadable);
        assert_eq!(*fetcher.calls.lock().unwrap(), 0);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn fetch_error_surfaces_as_failed_and_persists_nothing() {
        let (_g, store) = temp_store();
        let book = opds_book(Some("https://ex.org/a.epub"));
        let fetcher = FakeFetcher::failing("boom");
        let outcome = block_on(download_book_to_store(&store, &book, &fetcher));
        assert_eq!(
            outcome,
            DownloadOutcome::Failed { reason: "boom".into() }
        );
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn non_epub_bytes_rejected_before_persist() {
        let (_g, store) = temp_store();
        let book = opds_book(Some("https://ex.org/a.epub"));
        let fetcher = FakeFetcher::ok(b"<html>error page</html>".to_vec());
        let outcome = block_on(download_book_to_store(&store, &book, &fetcher));
        assert_eq!(outcome, DownloadOutcome::NotAnEpub);
        assert!(store.list().unwrap().is_empty());
        // Nothing written to the downloads dir either.
        let stray = fs::read_dir(store.dir())
            .map(|rd| rd.count())
            .unwrap_or(0);
        assert_eq!(stray, 0);
    }

    #[test]
    fn oversized_download_is_too_large_before_persist() {
        let (_g, store) = temp_store();
        let book = opds_book(Some("https://ex.org/a.epub"));
        // A buffer just over the cap; content need not be a real EPUB since the
        // size guard runs first.
        let big = vec![0u8; MAX_DOWNLOAD_BYTES + 1];
        let fetcher = FakeFetcher::ok(big);
        let outcome = block_on(download_book_to_store(&store, &book, &fetcher));
        assert!(matches!(outcome, DownloadOutcome::TooLarge { .. }));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn safe_filename_sanitizes_and_disambiguates() {
        let a = safe_filename("Bad/Name:Book?", "https://ex.org/a.epub");
        assert!(a.ends_with(".epub"));
        assert!(!a.contains('/') && !a.contains(':') && !a.contains('?'));
        // Same title, different URL → different file name (no collision).
        let b = safe_filename("Bad/Name:Book?", "https://ex.org/b.epub");
        assert_ne!(a, b);
    }
}
