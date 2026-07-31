//! LazyLibrarian connector.
//!
//! [LazyLibrarian](https://gitlab.com/LazyLibrarian/LazyLibrarian) is a
//! **self-hosted** book manager the user runs themselves. It exposes an official
//! REST API of the form `GET {base_url}/api?apikey={key}&cmd={command}`; simple
//! commands return the bare string `OK`, richer commands return JSON.
//!
//! With Readarr retired in June 2025, LazyLibrarian (and its forks) is the living
//! request/acquisition path. Libro talks **only to the user's own LazyLibrarian
//! instance** and bundles **no indexers or content sources** of its own — the
//! user configures those inside their own LazyLibrarian. Libro merely tells the
//! user's instance to do what it already does (see `ARCHITECTURE.md` → "Legal
//! boundaries").
//!
//! API reference: <https://lazylibrarian.gitlab.io/api/>
//!
//! Commands used:
//! * `getVersion`  — validate the base URL + api key (authenticate).
//! * `getAllBooks` — enumerate the user's books (CATALOG).
//! * `findBook`    — resolve a title/author to a LazyLibrarian `BookID` (search).
//! * `addBook`     — add a resolved book to the database.
//! * `queueBook`   — mark a book Wanted (REQUEST).
//! * `searchBook`  — trigger the user's own indexer search for a book (DOWNLOAD).
//!
//! Capabilities: [`ProviderCapabilities::CATALOG`],
//! [`ProviderCapabilities::REQUEST`], [`ProviderCapabilities::DOWNLOAD`].

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::models::{Book, MediaType};
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

/// Settings for the LazyLibrarian connector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LazyLibrarianConfig {
    /// Base URL of the user's LazyLibrarian instance, e.g.
    /// `http://192.168.1.10:5299`.
    pub base_url: String,
    /// API key from the LazyLibrarian settings (Config → Interface → API).
    pub api_key: String,
}

/// The media type to act on for request/queue operations.
///
/// LazyLibrarian tracks an ebook status and a separate audiobook status per
/// book; the `type` query parameter selects which one a command targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryType {
    Ebook,
    Audiobook,
}

impl LibraryType {
    /// The value of the `&type=` query parameter LazyLibrarian expects.
    pub fn as_param(self) -> &'static str {
        match self {
            LibraryType::Ebook => "eBook",
            LibraryType::Audiobook => "AudioBook",
        }
    }
}

/// The LazyLibrarian connector.
pub struct LazyLibrarianProvider {
    config: LazyLibrarianConfig,
    authenticated: bool,
    client: reqwest::Client,
}

impl LazyLibrarianProvider {
    pub const ID: &'static str = "lazylibrarian";

    pub fn new(config: LazyLibrarianConfig) -> Self {
        Self {
            config,
            authenticated: false,
            client: reqwest::Client::new(),
        }
    }

    /// Base URL with any trailing slash removed.
    fn base(&self) -> &str {
        self.config.base_url.trim_end_matches('/')
    }

    /// Issue a LazyLibrarian API command and return the raw response body.
    ///
    /// `extra` carries command-specific parameters (already unencoded; reqwest
    /// url-encodes them). The `apikey`, `cmd`, and `output=json` params are added
    /// automatically. Network/timeout failures and HTTP auth statuses are mapped
    /// to typed [`ProviderError`]s; the body is *not* interpreted here (some
    /// commands return the bare string `OK`, others return JSON or a plain error
    /// string) — see [`parse_ll_json`] / [`parse_ll_ok`].
    async fn get_raw(&self, cmd: &str, extra: &[(&str, &str)]) -> ProviderResult<String> {
        if self.base().is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }

        let mut query: Vec<(&str, &str)> = Vec::with_capacity(3 + extra.len());
        query.push(("apikey", self.config.api_key.as_str()));
        query.push(("cmd", cmd));
        // `output=json` is a no-op on stock LazyLibrarian (it always JSON-encodes
        // non-string data) but some forks honour it; harmless either way.
        query.push(("output", "json"));
        query.extend_from_slice(extra);

        let resp = self
            .client
            .get(format!("{}/api", self.base()))
            .query(&query)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Network(format!("timeout calling cmd={cmd}"))
                } else {
                    ProviderError::Network(e.to_string())
                }
            })?;

        match resp.status().as_u16() {
            // LazyLibrarian returns 200 even for its own error strings; those are
            // detected when the body is parsed.
            200 => resp
                .text()
                .await
                .map_err(|e| ProviderError::Network(e.to_string())),
            401 | 403 => Err(ProviderError::NotAuthenticated),
            other => Err(ProviderError::Other(format!(
                "unexpected status {other} from cmd={cmd}"
            ))),
        }
    }

    /// Run a command whose successful response is JSON of type `T`.
    async fn get_json<T: DeserializeOwned>(
        &self,
        cmd: &str,
        extra: &[(&str, &str)],
    ) -> ProviderResult<T> {
        let body = self.get_raw(cmd, extra).await?;
        parse_ll_json(&body)
    }

    /// Run a command whose successful response is `OK` (or a background sentinel).
    async fn get_ok(&self, cmd: &str, extra: &[(&str, &str)]) -> ProviderResult<()> {
        let body = self.get_raw(cmd, extra).await?;
        parse_ll_ok(&body)
    }

    /// Search LazyLibrarian's book API (`findBook`) for a title/author string.
    ///
    /// Returns the candidate books LazyLibrarian's configured book source
    /// (GoodReads/GoogleBooks) proposes, each carrying a `bookid` usable with
    /// [`request`](Self::request).
    pub async fn search(&self, query: &str) -> ProviderResult<Vec<LlSearchResult>> {
        self.get_json("findBook", &[("name", query)]).await
    }

    /// Resolve a [`Book`] to a LazyLibrarian `BookID`.
    ///
    /// Prefers an ISBN search when the book carries one, otherwise falls back to
    /// `title author`. Returns the first (best) match's id, if any.
    pub async fn resolve_book_id(&self, book: &Book) -> ProviderResult<Option<String>> {
        let query = if let Some(isbn) = book.identifiers.get("isbn") {
            isbn.clone()
        } else {
            let author = book.authors.first().cloned().unwrap_or_default();
            format!("{} {}", book.title, author).trim().to_string()
        };
        let results = self.search(&query).await?;
        Ok(results.into_iter().find_map(|r| r.bookid))
    }

    /// Request a book: add it to the user's instance, mark it Wanted, and trigger
    /// the instance's own indexer search.
    ///
    /// This is `addBook` → `queueBook` → `searchBook`. Libro performs **no**
    /// searching or downloading itself; it only instructs the user's LazyLibrarian
    /// (which owns all indexer/source configuration) to run its normal flow.
    pub async fn request(&self, book_id: &str, kind: LibraryType) -> ProviderResult<()> {
        let ty = kind.as_param();
        // Ensure the book exists in the database (no-op if already present).
        self.get_ok("addBook", &[("id", book_id)]).await?;
        // Mark it Wanted for the selected media type.
        self.get_ok("queueBook", &[("id", book_id), ("type", ty)])
            .await?;
        // Kick off the user's own configured search for it.
        self.get_ok("searchBook", &[("id", book_id), ("type", ty)])
            .await
    }
}

#[async_trait]
impl Provider for LazyLibrarianProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "LazyLibrarian"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::CATALOG
            | ProviderCapabilities::REQUEST
            | ProviderCapabilities::DOWNLOAD
    }

    async fn authenticate(&mut self, config: &serde_json::Value) -> ProviderResult<()> {
        if !config.is_null() {
            self.config = serde_json::from_value(config.clone())
                .map_err(|e| ProviderError::Config(e.to_string()))?;
        }
        if self.base().is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }

        // `getVersion` is a lightweight, side-effect-free command that still
        // requires a valid api key, so it doubles as a credential check.
        let _version: LlVersion = self.get_json("getVersion", &[]).await?;
        self.authenticated = true;
        Ok(())
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        if !self.authenticated {
            return Err(ProviderError::NotAuthenticated);
        }
        let rows: Vec<LlBook> = self.get_json("getAllBooks", &[]).await?;
        Ok(rows
            .iter()
            .map(|r| map_book_to_book(r, self.base(), Self::ID))
            .collect())
    }
}

// ---------------------------------------------------------------------------
// LazyLibrarian response types (only the fields Libro needs; rest ignored).
// ---------------------------------------------------------------------------

/// `getVersion` payload.
#[derive(Debug, Default, Deserialize)]
struct LlVersion {
    #[serde(default)]
    #[allow(dead_code)]
    install_type: String,
    #[serde(default)]
    #[allow(dead_code)]
    current_version: String,
}

/// A row from `getAllBooks`.
///
/// Field names mirror LazyLibrarian's `books`/`authors` columns. `getAllBooks`
/// returns the *ebook* `Status`; `AudioStatus` is not part of the stock query
/// but is accepted here so forks/extended responses that include it map an
/// audiobook to the right [`MediaType`].
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LlBook {
    #[serde(rename = "BookID", default)]
    pub book_id: String,
    #[serde(default)]
    pub book_name: String,
    #[serde(default)]
    pub author_name: String,
    /// Sub-title / series annotation, when present.
    #[serde(default)]
    pub book_sub: Option<String>,
    #[serde(default)]
    pub book_isbn: Option<String>,
    /// Cover image: either an absolute URL or an instance-relative cache path.
    #[serde(default)]
    pub book_img: Option<String>,
    /// eBook status: `Skipped` / `Wanted` / `Have` / `Open` / `Ignored` / …
    #[serde(default)]
    pub status: String,
    /// Audiobook status (present only on forks/extended responses).
    #[serde(default)]
    pub audio_status: Option<String>,
}

/// A candidate book from `findBook`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct LlSearchResult {
    pub bookid: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub bookname: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub authorname: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub bookisbn: Option<String>,
}

/// Shape of LazyLibrarian's JSON-object error form: `{"error": "..."}`.
#[derive(Debug, Deserialize)]
struct LlErrorObject {
    error: String,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without any network access).
// ---------------------------------------------------------------------------

/// Whether a LazyLibrarian status means the user owns/has the file.
fn is_owned(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "have" | "open"
    )
}

/// Choose the normalized [`MediaType`] for a row.
///
/// Treated as an audiobook only when it has an owned audio status but no owned
/// ebook status; otherwise defaults to ebook (the stock `getAllBooks` reports
/// the ebook status).
fn media_type_for(status: &str, audio_status: Option<&str>) -> MediaType {
    let audio_owned = audio_status.map(is_owned).unwrap_or(false);
    if audio_owned && !is_owned(status) {
        MediaType::Audiobook
    } else {
        MediaType::Ebook
    }
}

/// Build a usable cover URL from LazyLibrarian's `BookImg`.
fn cover_url(base_url: &str, book_img: Option<&str>) -> Option<String> {
    let img = book_img?.trim();
    if img.is_empty() {
        return None;
    }
    if img.starts_with("http://") || img.starts_with("https://") {
        Some(img.to_string())
    } else {
        Some(format!("{}/{}", base_url, img.trim_start_matches('/')))
    }
}

/// Map one `getAllBooks` row into a normalized [`Book`].
fn map_book_to_book(row: &LlBook, base_url: &str, source_provider_id: &str) -> Book {
    let mut book = Book::new(
        row.book_id.clone(),
        if row.book_name.is_empty() {
            "Untitled".to_string()
        } else {
            row.book_name.clone()
        },
        media_type_for(&row.status, row.audio_status.as_deref()),
        source_provider_id,
    );

    book.authors = row
        .author_name
        .split(',')
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();

    book.series = row
        .book_sub
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    book.cover_url = cover_url(base_url, row.book_img.as_deref());

    if let Some(isbn) = row.book_isbn.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        book.identifiers.insert("isbn".into(), isbn.to_string());
    }
    // With the GoodReads book source, `BookID` is the numeric GoodReads id.
    if !row.book_id.is_empty() && row.book_id.chars().all(|c| c.is_ascii_digit()) {
        book.identifiers
            .insert("goodreads".into(), row.book_id.clone());
    }

    book
}

/// Classify a bare (non-JSON) LazyLibrarian response body.
///
/// LazyLibrarian returns plain, unquoted strings for `OK`, and for its own error
/// messages (`Incorrect API key`, `Missing parameter: id`, `Invalid id: …`, …).
/// Auth-related messages become [`ProviderError::NotAuthenticated`]; everything
/// else non-`OK` becomes [`ProviderError::Api`].
fn classify_plain(body: &str) -> Result<(), ProviderError> {
    let trimmed = body.trim().trim_matches('"');
    // Background/void commands may return `OK` or a JSON `null`.
    if trimmed.eq_ignore_ascii_case("ok") || trimmed.eq_ignore_ascii_case("null") || trimmed.is_empty() {
        return Ok(());
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("api key") || lower.contains("api not enabled") {
        return Err(ProviderError::NotAuthenticated);
    }
    Err(ProviderError::Api(trimmed.to_string()))
}

/// Parse a JSON response of type `T`, surfacing LazyLibrarian's string/object
/// error forms as typed errors.
fn parse_ll_json<T: DeserializeOwned>(body: &str) -> ProviderResult<T> {
    // Happy path: the expected JSON shape.
    if let Ok(value) = serde_json::from_str::<T>(body) {
        return Ok(value);
    }
    // `{"error": "..."}` object form.
    if let Ok(err) = serde_json::from_str::<LlErrorObject>(body) {
        let lower = err.error.to_ascii_lowercase();
        if lower.contains("api key") || lower.contains("api not enabled") {
            return Err(ProviderError::NotAuthenticated);
        }
        return Err(ProviderError::Api(err.error));
    }
    // Bare string form (`OK` or a plain error message).
    match classify_plain(body) {
        // `OK` where structured data was expected is itself unexpected.
        Ok(()) => Err(ProviderError::Api(format!(
            "expected JSON but got: {}",
            body.trim()
        ))),
        Err(e) => Err(e),
    }
}

/// Parse a response whose success form is `OK` (or a background sentinel).
fn parse_ll_ok(body: &str) -> ProviderResult<()> {
    if let Ok(err) = serde_json::from_str::<LlErrorObject>(body) {
        let lower = err.error.to_ascii_lowercase();
        if lower.contains("api key") || lower.contains("api not enabled") {
            return Err(ProviderError::NotAuthenticated);
        }
        return Err(ProviderError::Api(err.error));
    }
    classify_plain(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://ll.local:5299";

    fn version_fixture() -> &'static str {
        r#"{
          "install_type": "git",
          "current_version": "abc123",
          "latest_version": "abc123",
          "commits_behind": 0
        }"#
    }

    // Representative `getAllBooks` payload: one owned ebook and one owned
    // audiobook (the latter via an extended `AudioStatus` field).
    fn getallbooks_fixture() -> &'static str {
        r#"[
          {
            "AuthorID": "1234",
            "AuthorName": "Ursula K. Le Guin, Co Author",
            "BookName": "A Wizard of Earthsea",
            "BookSub": "Earthsea Cycle",
            "BookIsbn": "9780553383041",
            "BookImg": "cache/book/9780553383041.jpg",
            "BookLink": "https://www.goodreads.com/book/show/13642.html",
            "BookID": "13642",
            "Status": "Open"
          },
          {
            "AuthorID": "555",
            "AuthorName": "Andy Weir",
            "BookName": "Project Hail Mary",
            "BookIsbn": "",
            "BookImg": "https://covers.example/phm.jpg",
            "BookID": "googImg42x",
            "Status": "Skipped",
            "AudioStatus": "Have"
          }
        ]"#
    }

    fn findbook_fixture() -> &'static str {
        r#"[
          {
            "bookid": "13642",
            "bookname": "A Wizard of Earthsea",
            "authorname": "Ursula K. Le Guin",
            "bookisbn": "9780553383041"
          },
          {
            "bookid": "99999",
            "bookname": "A Wizard of Earthsea (annotated)",
            "authorname": "Ursula K. Le Guin"
          }
        ]"#
    }

    #[test]
    fn parses_version_payload_for_authenticate() {
        let v: LlVersion = parse_ll_json(version_fixture()).unwrap();
        assert_eq!(v.install_type, "git");
        assert_eq!(v.current_version, "abc123");
    }

    #[test]
    fn maps_getallbooks_ebook_and_audiobook_rows() {
        let rows: Vec<LlBook> = parse_ll_json(getallbooks_fixture()).unwrap();
        let books: Vec<Book> = rows
            .iter()
            .map(|r| map_book_to_book(r, BASE, LazyLibrarianProvider::ID))
            .collect();

        assert_eq!(books.len(), 2);

        // Ebook row.
        let ebook = &books[0];
        assert_eq!(ebook.title, "A Wizard of Earthsea");
        assert_eq!(ebook.media_type, MediaType::Ebook);
        assert_eq!(ebook.authors, vec!["Ursula K. Le Guin", "Co Author"]);
        assert_eq!(ebook.series.as_deref(), Some("Earthsea Cycle"));
        assert_eq!(
            ebook.cover_url.as_deref(),
            Some("http://ll.local:5299/cache/book/9780553383041.jpg")
        );
        assert_eq!(ebook.identifiers.get("isbn").map(String::as_str), Some("9780553383041"));
        assert_eq!(ebook.identifiers.get("goodreads").map(String::as_str), Some("13642"));
        assert_eq!(ebook.source_provider_id, "lazylibrarian");

        // Audiobook row: owned audio status, absolute cover, non-numeric id.
        let audio = &books[1];
        assert_eq!(audio.media_type, MediaType::Audiobook);
        assert_eq!(audio.cover_url.as_deref(), Some("https://covers.example/phm.jpg"));
        assert!(audio.identifiers.get("isbn").is_none());
        assert!(audio.identifiers.get("goodreads").is_none());
    }

    #[test]
    fn resolves_first_book_id_from_findbook_results() {
        let results: Vec<LlSearchResult> = parse_ll_json(findbook_fixture()).unwrap();
        let first_id = results.into_iter().find_map(|r| r.bookid);
        assert_eq!(first_id.as_deref(), Some("13642"));
    }

    #[test]
    fn plain_ok_body_is_success() {
        assert!(parse_ll_ok("OK").is_ok());
        assert!(parse_ll_ok("null").is_ok());
    }

    #[test]
    fn plain_error_string_becomes_typed_error() {
        // Auth errors → NotAuthenticated.
        assert!(matches!(
            parse_ll_ok("Incorrect API key"),
            Err(ProviderError::NotAuthenticated)
        ));
        // Other errors → Api.
        match parse_ll_ok("Invalid id: 42") {
            Err(ProviderError::Api(msg)) => assert_eq!(msg, "Invalid id: 42"),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn error_object_form_becomes_api_error() {
        let err = parse_ll_json::<Vec<LlBook>>(r#"{"error": "database is locked"}"#);
        match err {
            Err(ProviderError::Api(msg)) => assert_eq!(msg, "database is locked"),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn library_type_maps_to_query_param() {
        assert_eq!(LibraryType::Ebook.as_param(), "eBook");
        assert_eq!(LibraryType::Audiobook.as_param(), "AudioBook");
    }
}
