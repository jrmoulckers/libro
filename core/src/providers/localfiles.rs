//! Local Files / personal EPUB connector.
//!
//! This connector catalogs the user's **own DRM-free EPUB files on disk** — the
//! most unambiguously legal source there is. It recursively scans the configured
//! folders for `.epub` files and parses each one's OPF package document into a
//! normalized [`Book`].
//!
//! **No DRM handling.** Libro never attempts to decrypt anything. If a file is
//! DRM-protected (e.g. Adobe ADEPT) we read whatever open metadata is available
//! and otherwise skip it — decryption/circumvention is explicitly out of scope
//! (see `ARCHITECTURE.md` → "Legal boundaries").
//!
//! Capability: [`ProviderCapabilities::CATALOG`].
//!
//! ## Parsing approach
//! We use [`zip`] to read the EPUB container and [`roxmltree`] (a small read-only
//! XML DOM) to parse `META-INF/container.xml` and the OPF. This keeps the
//! dependency surface light and gives us direct control over the few OPF fields
//! Libro cares about — the Dublin Core metadata, the Calibre `series` `<meta>`,
//! and the `cover-image` manifest property — which the higher-level `epub` crate
//! does not expose as conveniently.
//!
//! ## Covers in a pure-client app
//! EPUB covers are bytes inside the zip, not URLs. When a book has an embedded
//! cover we set [`Book::cover_url`] to a `localcover://{book_id}` reference; the
//! frontend resolves it through the `get_local_cover` Tauri command, which calls
//! [`extract_cover`]. When a book has **no** embedded cover we leave `cover_url`
//! `None` so the metadata-enrichment pass can fill it from Open Library.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::{Book, MediaType};
use crate::providers::{Provider, ProviderCapabilities, ProviderResult};

const DC_NS: &str = "http://purl.org/dc/elements/1.1/";

/// Settings for the Local Files connector: one or more folders to scan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalFilesConfig {
    /// Absolute paths to folders that hold the user's EPUB files. Each is scanned
    /// recursively.
    #[serde(default)]
    pub library_paths: Vec<PathBuf>,
}

/// The Local Files connector.
pub struct LocalFilesProvider {
    config: LocalFilesConfig,
}

impl LocalFilesProvider {
    pub const ID: &'static str = "localfiles";

    pub fn new(config: LocalFilesConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Provider for LocalFilesProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "Local Files"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::CATALOG
    }

    /// No credentials — local files. This never fails on missing folders; those
    /// are reported per-path during [`list_library`](Self::list_library).
    async fn authenticate(&mut self, _config: &serde_json::Value) -> ProviderResult<()> {
        Ok(())
    }

    /// Recursively scan every configured folder for `.epub` files and parse each
    /// into a [`Book`].
    ///
    /// Robustness: a nonexistent path, an unreadable directory, or a single
    /// corrupt/non-EPUB file is logged and skipped — never a panic and never an
    /// aborted scan. Missing metadata fields become `None`/empty, not errors.
    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        if self.config.library_paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut books = Vec::new();
        for root in &self.config.library_paths {
            if !root.exists() {
                eprintln!(
                    "libro: localfiles path does not exist, skipping: {}",
                    root.display()
                );
                continue;
            }
            let mut epubs = Vec::new();
            collect_epubs(root, &mut epubs);
            for path in epubs {
                match parse_epub_file(&path) {
                    Ok(book) => books.push(book),
                    Err(e) => eprintln!("libro: skipping '{}': {e}", path.display()),
                }
            }
        }
        Ok(books)
    }
}

/// Extract the raw cover-image bytes for the local book identified by `book_id`.
///
/// Walks the configured library paths (cheaply — only directory names, no XML
/// parsing — until the id matches), then opens just that one EPUB and returns its
/// cover bytes. Returns `None` if the book isn't found or has no cover. Because it
/// only ever reads files *under* the configured folders, it can't be tricked into
/// reading arbitrary paths.
pub fn extract_cover(config: &LocalFilesConfig, book_id: &str) -> Option<Vec<u8>> {
    for root in &config.library_paths {
        if !root.exists() {
            continue;
        }
        let mut epubs = Vec::new();
        collect_epubs(root, &mut epubs);
        for path in epubs {
            let abs = canonical(&path);
            if book_id_for(&abs) == book_id {
                return read_cover_bytes(&abs);
            }
        }
    }
    None
}

/// Read the full EPUB file bytes for the local book identified by `book_id`.
///
/// Like [`extract_cover`], this resolves the id by walking the configured library
/// paths and matching the path hash — so it can only ever return a file that
/// lives *under* a configured folder. A `book_id` that doesn't match any scanned
/// file yields `None`; there is no way to make it read an arbitrary path, because
/// the id is never used to construct a path (only compared against hashes of
/// paths we discovered ourselves).
pub fn read_book_file(config: &LocalFilesConfig, book_id: &str) -> Option<Vec<u8>> {
    for root in &config.library_paths {
        if !root.exists() {
            continue;
        }
        let mut epubs = Vec::new();
        collect_epubs(root, &mut epubs);
        for path in epubs {
            let abs = canonical(&path);
            if book_id_for(&abs) == book_id {
                return fs::read(&abs).ok();
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Filesystem scanning
// ---------------------------------------------------------------------------

/// Recursively collect `.epub` files under `dir` (case-insensitive extension).
fn collect_epubs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("libro: cannot read dir '{}': {e}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_epubs(&path, out);
        } else if is_epub(&path) {
            out.push(path);
        }
    }
}

fn is_epub(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("epub"))
        .unwrap_or(false)
}

/// Content-based EPUB sniff: does this byte buffer look like a real EPUB?
///
/// Unlike [`is_epub`] (which only checks the file *extension*), this opens the
/// bytes as a ZIP and verifies the presence of `META-INF/container.xml` — the
/// mandatory EPUB entry. It is used to validate **downloaded** bytes before we
/// persist them, so an HTML error page or a truncated download is rejected as
/// [`crate::downloads::DownloadOutcome::NotAnEpub`] instead of landing on disk.
pub fn is_epub_bytes(bytes: &[u8]) -> bool {
    let cursor = std::io::Cursor::new(bytes);
    match zip::ZipArchive::new(cursor) {
        Ok(mut zip) => zip.by_name("META-INF/container.xml").is_ok(),
        Err(_) => false,
    }
}

/// Public wrapper over the stable path-hash id used by [`read_book_file`] and
/// [`extract_cover`], canonicalizing `path` first. The download store records
/// this id so a downloaded EPUB resolves through the exact same reader/cover
/// path as any other locally-scanned book.
pub fn book_id_for_path(path: &Path) -> String {
    book_id_for(&canonical(path))
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

// ---------------------------------------------------------------------------
// EPUB → Book
// ---------------------------------------------------------------------------

/// Open and parse a single EPUB into a [`Book`], or an error describing why it
/// was skipped.
fn parse_epub_file(path: &Path) -> Result<Book, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a valid zip/EPUB: {e}"))?;

    let container =
        read_zip_string(&mut zip, "META-INF/container.xml").ok_or("missing META-INF/container.xml")?;
    let opf_path = parse_container(&container).ok_or("no OPF rootfile in container.xml")?;
    let opf_xml =
        read_zip_string(&mut zip, &opf_path).ok_or_else(|| format!("missing OPF at {opf_path}"))?;

    let parsed = parse_opf(&opf_xml);

    // A cover counts only if the referenced entry actually exists in the zip.
    let has_cover = parsed
        .cover_href
        .as_ref()
        .map(|href| zip.by_name(&resolve_relative(&opf_path, href)).is_ok())
        .unwrap_or(false);

    Ok(opf_to_book(&parsed, &canonical(path), has_cover))
}

fn read_zip_string<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

fn read_cover_bytes(path: &Path) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let container = read_zip_string(&mut zip, "META-INF/container.xml")?;
    let opf_path = parse_container(&container)?;
    let opf_xml = read_zip_string(&mut zip, &opf_path)?;
    let href = parse_opf(&opf_xml).cover_href?;
    let full = resolve_relative(&opf_path, &href);
    let mut entry = zip.by_name(&full).ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Parsed subset of an OPF package document.
#[derive(Debug, Default)]
struct ParsedOpf {
    title: String,
    authors: Vec<String>,
    language: Option<String>,
    publisher: Option<String>,
    description: Option<String>,
    isbn13: Option<String>,
    isbn10: Option<String>,
    /// Non-ISBN identifiers, keyed by their scheme (or `identifier`).
    other_ids: BTreeMap<String, String>,
    series: Option<String>,
    /// Cover href, relative to the OPF's own directory.
    cover_href: Option<String>,
}

/// Find the OPF rootfile path from `META-INF/container.xml`.
fn parse_container(xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .map(|s| s.to_string())
}

/// Parse the OPF metadata + manifest into a [`ParsedOpf`].
fn parse_opf(xml: &str) -> ParsedOpf {
    let mut p = ParsedOpf::default();
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(_) => return p,
    };

    // Dublin Core metadata.
    for node in doc.descendants().filter(|n| n.is_element() && is_dc(n)) {
        match node.tag_name().name() {
            "title" if p.title.is_empty() => p.title = text(&node),
            "creator" => {
                let name = text(&node);
                if !name.is_empty() {
                    p.authors.push(name);
                }
            }
            "language" => p.language = p.language.take().or_else(|| nonempty(text(&node))),
            "publisher" => p.publisher = p.publisher.take().or_else(|| nonempty(text(&node))),
            "description" => {
                p.description = p.description.take().or_else(|| nonempty(text(&node)))
            }
            "identifier" => classify_identifier(&node, &mut p),
            _ => {}
        }
    }

    // Calibre series + legacy cover pointer live in <meta> elements.
    let mut legacy_cover_id: Option<String> = None;
    for node in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "meta")
    {
        match node.attribute("name") {
            Some("calibre:series") => {
                p.series = node.attribute("content").and_then(|s| nonempty(s.to_string()))
            }
            Some("cover") => {
                legacy_cover_id = node.attribute("content").map(|s| s.to_string());
            }
            _ => {}
        }
    }

    p.cover_href = resolve_cover_href(&doc, legacy_cover_id.as_deref());
    p
}

fn is_dc(node: &roxmltree::Node) -> bool {
    node.tag_name().namespace() == Some(DC_NS)
}

fn text(node: &roxmltree::Node) -> String {
    node.text().unwrap_or("").trim().to_string()
}

fn nonempty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Sort a `dc:identifier` into an ISBN slot or the `other_ids` bag.
fn classify_identifier(node: &roxmltree::Node, p: &mut ParsedOpf) {
    let raw = text(node);
    if raw.is_empty() {
        return;
    }
    // `opf:scheme` is namespaced; match by local attribute name.
    let scheme = node
        .attributes()
        .find(|a| a.name() == "scheme")
        .map(|a| a.value().to_string());

    let declared_isbn = scheme
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("ISBN"))
        .unwrap_or(false)
        || raw.to_ascii_lowercase().contains("isbn");

    let digits = extract_isbn_digits(&raw);
    if (declared_isbn || is_isbn_shaped(&digits)) && !digits.is_empty() {
        match digits.len() {
            13 => {
                p.isbn13.get_or_insert(digits);
                return;
            }
            10 => {
                p.isbn10.get_or_insert(digits);
                return;
            }
            _ => {}
        }
    }
    let key = scheme
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "identifier".to_string());
    p.other_ids.entry(key).or_insert(raw);
}

/// Strip `urn:isbn:` and separators, upper-casing the ISBN-10 check char `X`.
fn extract_isbn_digits(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let body = lower.strip_prefix("urn:isbn:").unwrap_or(&lower);
    body.chars()
        .filter(|c| c.is_ascii_digit() || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn is_isbn_shaped(digits: &str) -> bool {
    (digits.len() == 13 && digits.chars().all(|c| c.is_ascii_digit())) || digits.len() == 10
}

/// Resolve the cover href from the manifest: prefer the EPUB3
/// `properties="cover-image"` item, else the legacy `<meta name="cover">` id.
fn resolve_cover_href(doc: &roxmltree::Document, legacy_id: Option<&str>) -> Option<String> {
    let items: Vec<roxmltree::Node> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "item")
        .collect();

    if let Some(item) = items.iter().find(|n| {
        n.attribute("properties")
            .map(|p| p.split_whitespace().any(|t| t == "cover-image"))
            .unwrap_or(false)
    }) {
        return item.attribute("href").map(|s| s.to_string());
    }
    if let Some(id) = legacy_id {
        if let Some(item) = items.iter().find(|n| n.attribute("id") == Some(id)) {
            return item.attribute("href").map(|s| s.to_string());
        }
    }
    None
}

/// Join a manifest href (relative to the OPF's directory) into a full zip path,
/// collapsing `.` and `..` segments.
fn resolve_relative(opf_path: &str, href: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    // Start from the OPF's parent directory.
    let base = opf_path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    for seg in base.split('/').chain(href.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

/// Build the normalized [`Book`] from the parsed OPF.
fn opf_to_book(p: &ParsedOpf, abs_path: &Path, has_cover: bool) -> Book {
    let id = book_id_for(abs_path);
    let title = if p.title.trim().is_empty() {
        abs_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string()
    } else {
        p.title.clone()
    };

    let mut book = Book::new(id.clone(), title, MediaType::Ebook, LocalFilesProvider::ID);
    book.authors = p.authors.clone();
    book.series = p.series.clone();
    book.description = p.description.clone();
    if let Some(isbn) = &p.isbn13 {
        book.identifiers.insert("isbn13".into(), isbn.clone());
    }
    if let Some(isbn) = &p.isbn10 {
        book.identifiers.insert("isbn10".into(), isbn.clone());
    }
    for (k, v) in &p.other_ids {
        book.identifiers.entry(k.clone()).or_insert_with(|| v.clone());
    }
    if has_cover {
        book.cover_url = Some(format!("localcover://{id}"));
    }
    book
}

/// Stable per-file id: FNV-1a of the canonical path (deterministic across runs,
/// no extra crypto dependency).
fn book_id_for(path: &Path) -> String {
    format!("{:016x}", fnv1a_64(path.to_string_lossy().as_bytes()))
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Provider;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    fn opf(title: &str, isbn: Option<&str>, series: Option<&str>, cover: bool) -> String {
        let isbn_line = isbn
            .map(|i| format!(r#"<dc:identifier opf:scheme="ISBN">{i}</dc:identifier>"#))
            .unwrap_or_default();
        let series_line = series
            .map(|s| format!(r#"<meta name="calibre:series" content="{s}"/>"#))
            .unwrap_or_default();
        let (cover_meta, cover_item) = if cover {
            (
                r#"<meta name="cover" content="cover-img"/>"#,
                r#"<item id="cover-img" href="cover.png" media-type="image/png" properties="cover-image"/>"#,
            )
        } else {
            ("", "")
        };
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>{title}</dc:title>
    <dc:creator>Joshua Bloch</dc:creator>
    <dc:creator>Second Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:publisher>Addison-Wesley</dc:publisher>
    <dc:description>A description straight from the EPUB.</dc:description>
    {isbn_line}
    {series_line}
    {cover_meta}
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
    {cover_item}
  </manifest>
  <spine><itemref idref="content"/></spine>
</package>"#
        )
    }

    fn write_epub(dir: &Path, name: &str, opf_xml: &str, cover: bool) -> PathBuf {
        let path = dir.join(name);
        let file = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", stored).unwrap();
        zip.write_all(CONTAINER_XML.as_bytes()).unwrap();
        zip.start_file("OEBPS/content.opf", stored).unwrap();
        zip.write_all(opf_xml.as_bytes()).unwrap();
        if cover {
            zip.start_file("OEBPS/cover.png", stored).unwrap();
            zip.write_all(b"PNG-COVER-BYTES").unwrap();
        }
        zip.finish().unwrap();
        path
    }

    async fn scan(dir: &Path) -> Vec<Book> {
        let provider = LocalFilesProvider::new(LocalFilesConfig {
            library_paths: vec![dir.to_path_buf()],
        });
        provider.list_library().await.unwrap()
    }

    #[tokio::test]
    async fn parses_generated_epub_into_book() {
        let tmp = tempfile::tempdir().unwrap();
        write_epub(
            tmp.path(),
            "effective-java.epub",
            &opf("Effective Java", Some("9780134685991"), Some("The Java Series"), true),
            true,
        );

        let books = scan(tmp.path()).await;
        assert_eq!(books.len(), 1);
        let b = &books[0];
        assert_eq!(b.title, "Effective Java");
        assert_eq!(b.authors, vec!["Joshua Bloch", "Second Author"]);
        assert_eq!(b.media_type, MediaType::Ebook);
        assert_eq!(b.source_provider_id, "localfiles");
        assert_eq!(b.series.as_deref(), Some("The Java Series"));
        assert_eq!(b.description.as_deref(), Some("A description straight from the EPUB."));
        assert_eq!(b.identifiers.get("isbn13").map(String::as_str), Some("9780134685991"));
        assert_eq!(b.cover_url.as_deref(), Some(format!("localcover://{}", b.id).as_str()));
    }

    #[tokio::test]
    async fn epub_without_cover_leaves_cover_url_none() {
        let tmp = tempfile::tempdir().unwrap();
        write_epub(
            tmp.path(),
            "no-cover.epub",
            &opf("No Cover Book", Some("9780134685991"), None, false),
            false,
        );

        let books = scan(tmp.path()).await;
        assert_eq!(books.len(), 1);
        // No embedded cover -> None, so the enrichment pass can fill it later.
        assert!(books[0].cover_url.is_none());
        assert!(books[0].series.is_none());
    }

    #[tokio::test]
    async fn corrupt_and_non_epub_files_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        // One good EPUB.
        write_epub(tmp.path(), "good.epub", &opf("Good", Some("9780134685991"), None, false), false);
        // A .txt file (wrong extension) — ignored by the scan entirely.
        fs::write(tmp.path().join("notes.txt"), b"just some text").unwrap();
        // A file with a .epub extension that is NOT a zip — skipped with a log.
        fs::write(tmp.path().join("broken.epub"), b"this is not a zip archive").unwrap();

        let books = scan(tmp.path()).await;
        assert_eq!(books.len(), 1, "only the valid EPUB should be catalogued");
        assert_eq!(books[0].title, "Good");
    }

    #[tokio::test]
    async fn nonexistent_path_yields_empty_without_panic() {
        let provider = LocalFilesProvider::new(LocalFilesConfig {
            library_paths: vec![PathBuf::from("/no/such/libro/path/hopefully")],
        });
        let books = provider.list_library().await.unwrap();
        assert!(books.is_empty());
    }

    #[tokio::test]
    async fn no_configured_paths_is_empty() {
        let provider = LocalFilesProvider::new(LocalFilesConfig::default());
        assert!(provider.list_library().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn extract_cover_returns_embedded_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        write_epub(tmp.path(), "withcover.epub", &opf("C", Some("9780134685991"), None, true), true);

        let cfg = LocalFilesConfig {
            library_paths: vec![tmp.path().to_path_buf()],
        };
        let books = scan(tmp.path()).await;
        let bytes = extract_cover(&cfg, &books[0].id).expect("cover bytes");
        assert_eq!(bytes, b"PNG-COVER-BYTES");

        // Unknown id -> None, no panic.
        assert!(extract_cover(&cfg, "deadbeefdeadbeef").is_none());
    }

    #[tokio::test]
    async fn read_book_file_returns_epub_bytes_and_rejects_unknown_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_epub(tmp.path(), "read.epub", &opf("Readable", Some("9780134685991"), None, false), false);

        let cfg = LocalFilesConfig {
            library_paths: vec![tmp.path().to_path_buf()],
        };
        let books = scan(tmp.path()).await;
        let bytes = read_book_file(&cfg, &books[0].id).expect("epub bytes");
        // Real EPUB bytes: a zip starts with the local-file-header magic "PK\x03\x04".
        assert_eq!(&bytes[..2], b"PK");
        assert!(bytes.len() > 100);

        // An id that matches no scanned file yields None — no path-escape possible,
        // since the id is only ever compared against hashes of paths we discovered.
        assert!(read_book_file(&cfg, "0000000000000000").is_none());
    }

    #[test]
    fn recursive_scan_finds_nested_epubs() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        write_epub(&nested, "deep.epub", &opf("Deep", None, None, false), false);

        let mut out = Vec::new();
        collect_epubs(tmp.path(), &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn parses_urn_isbn_and_calibre_series_from_opf() {
        let xml = opf("T", Some("urn:isbn:978-0-13-468599-1"), Some("My Series"), false);
        let parsed = parse_opf(&xml);
        assert_eq!(parsed.isbn13.as_deref(), Some("9780134685991")); // hyphens/urn stripped
        assert_eq!(parsed.series.as_deref(), Some("My Series"));
        assert_eq!(parsed.authors.len(), 2);
    }

    #[test]
    fn isbn10_and_non_isbn_identifier_are_classified() {
        let mut p = ParsedOpf::default();
        // Build a tiny OPF with an ISBN-10 and a UUID identifier.
        let xml = r#"<package xmlns="http://www.idpf.org/2007/opf">
          <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
            <dc:identifier opf:scheme="ISBN">0-13-468599-7</dc:identifier>
            <dc:identifier opf:scheme="UUID">urn:uuid:12345</dc:identifier>
          </metadata>
        </package>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        for n in doc.descendants().filter(|n| n.is_element() && is_dc(n) && n.tag_name().name() == "identifier") {
            classify_identifier(&n, &mut p);
        }
        assert_eq!(p.isbn10.as_deref(), Some("0134685997")); // hyphens stripped, 10 digits kept
        assert!(p.other_ids.contains_key("uuid"));
    }

    #[test]
    fn resolve_relative_collapses_dot_segments() {
        assert_eq!(resolve_relative("OEBPS/content.opf", "cover.png"), "OEBPS/cover.png");
        assert_eq!(resolve_relative("OEBPS/content.opf", "images/c.png"), "OEBPS/images/c.png");
        assert_eq!(resolve_relative("OEBPS/content.opf", "../cover.png"), "cover.png");
        assert_eq!(resolve_relative("content.opf", "cover.png"), "cover.png");
    }
}
