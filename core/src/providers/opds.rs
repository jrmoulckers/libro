//! Generic OPDS catalog connector.
//!
//! [OPDS](https://specs.opds.io/) (Open Publication Distribution System) is a
//! widely-reused open standard built on Atom. Because it's a *standard*, one
//! connector reaches many servers: **Calibre-Web**, **Calibre**'s built-in
//! content server, **Kavita**, **Komga**, and public/public-domain catalogs such
//! as Standard Ebooks and Project Gutenberg.
//!
//! Legal basis: the user points Libro at **their own OPDS server** or at a
//! **public** OPDS feed. Libro hosts nothing and, as everywhere else, performs
//! **no DRM handling** — it only reads the feed and, on request, fetches a file
//! the server already offers.
//!
//! ## Scope / dialect
//! This targets **OPDS 1.2 (Atom/XML)** — the dialect Calibre-Web and most
//! self-hosted servers emit. OPDS 2.0 (JSON) is a TODO (see bottom of file).
//!
//! ## Feed model
//! An OPDS feed is either:
//! * a **navigation feed** — its entries link to *sub-catalogs*
//!   (`rel="subsection"` / `type=application/atom+xml;profile=opds-catalog`), or
//! * an **acquisition feed** — its entries are *books*, each carrying one or more
//!   acquisition links (`rel="http://opds-spec.org/acquisition"…`).
//!
//! From the configured `feed_url` the connector auto-discovers books: it follows
//! navigation entries down to acquisition feeds and follows `rel="next"`
//! pagination, **bounded** by a small depth + total-page cap so a large catalog
//! can't run away.
//!
//! Capabilities: [`ProviderCapabilities::CATALOG`], [`ProviderCapabilities::DOWNLOAD`].

use std::collections::{HashSet, VecDeque};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::{Book, MediaType};
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

/// How many nested navigation levels the crawler will descend.
const MAX_DEPTH: u32 = 3;
/// Hard cap on total feed documents fetched in one `list_library` run.
const MAX_PAGES: u32 = 25;

const REL_ACQUISITION_PREFIX: &str = "http://opds-spec.org/acquisition";
const REL_IMAGE: &str = "http://opds-spec.org/image";
const REL_THUMBNAIL: &str = "http://opds-spec.org/image/thumbnail";

const USER_AGENT: &str = concat!("Libro/", env!("CARGO_PKG_VERSION"), " (OPDS connector)");

/// Settings for the OPDS connector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpdsConfig {
    /// The OPDS feed URL to start from — either a navigation root (e.g.
    /// `https://example/opds`) or a direct acquisition feed.
    pub feed_url: String,
    /// Optional HTTP Basic auth username (Calibre-Web's default auth). Omit for
    /// unauthenticated public feeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Optional HTTP Basic auth password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// A single acquisition link parsed from an OPDS entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Acquisition {
    /// Absolute URL of the downloadable file.
    pub href: String,
    /// MIME type, e.g. `application/epub+zip`.
    pub mime: Option<String>,
    /// The full `rel`, e.g. `…/acquisition/open-access`.
    pub rel: String,
}

/// The result of parsing one feed document.
#[derive(Debug, Default)]
pub struct ParsedFeed {
    /// Books mapped from acquisition entries.
    pub books: Vec<Book>,
    /// Absolute URLs of sub-catalogs referenced by navigation entries.
    pub nav_hrefs: Vec<String>,
    /// Absolute URL of the `rel="next"` pagination link, if any.
    pub next: Option<String>,
}

impl ParsedFeed {
    /// True when this document is an acquisition feed (has book entries).
    pub fn is_acquisition(&self) -> bool {
        !self.books.is_empty()
    }
}

/// The OPDS connector.
pub struct OpdsProvider {
    config: OpdsConfig,
    client: reqwest::Client,
}

impl OpdsProvider {
    pub const ID: &'static str = "opds";

    pub fn new(config: OpdsConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Perform an authenticated GET, returning the raw [`reqwest::Response`].
    async fn send(&self, url: &str) -> ProviderResult<reqwest::Response> {
        let mut req = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(
                reqwest::header::ACCEPT,
                "application/atom+xml, application/xml;q=0.9, */*;q=0.5",
            );
        if let Some(user) = &self.config.username {
            req = req.basic_auth(user, self.config.password.as_deref());
        }

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ProviderError::Network(format!("timeout requesting {url}"))
            } else {
                ProviderError::Network(e.to_string())
            }
        })?;

        match resp.status().as_u16() {
            200..=299 => Ok(resp),
            401 | 403 => Err(ProviderError::NotAuthenticated),
            404 => Err(ProviderError::Api(format!("feed not found: {url}"))),
            other => Err(ProviderError::Api(format!(
                "unexpected HTTP {other} from {url}"
            ))),
        }
    }

    /// Fetch a feed document's XML body.
    async fn fetch_text(&self, url: &str) -> ProviderResult<String> {
        self.send(url)
            .await?
            .text()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))
    }

    /// Download the raw bytes of an acquisition URL (DOWNLOAD capability).
    ///
    /// Uses the same auth as catalog requests. This is the low-level fetch; disk
    /// persistence and a download-manager UI are a later phase (see TODOs).
    pub async fn download_url(&self, url: &str) -> ProviderResult<Vec<u8>> {
        let bytes = self
            .send(url)
            .await?
            .bytes()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    /// Download a [`Book`] previously produced by this connector, using the
    /// acquisition link captured in its identifiers.
    pub async fn download_book(&self, book: &Book) -> ProviderResult<Vec<u8>> {
        let url = book
            .identifiers
            .get("opds:acquisition_url")
            .ok_or_else(|| ProviderError::Unsupported)?;
        self.download_url(url).await
    }
}

#[async_trait]
impl Provider for OpdsProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "OPDS"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::CATALOG | ProviderCapabilities::DOWNLOAD
    }

    /// Validate the feed URL + credentials by fetching the configured feed and
    /// confirming it parses as an OPDS Atom feed.
    async fn authenticate(&mut self, _config: &serde_json::Value) -> ProviderResult<()> {
        if self.config.feed_url.trim().is_empty() {
            return Err(ProviderError::Config("feed_url is empty".into()));
        }
        let xml = self.fetch_text(&self.config.feed_url).await?;
        // parse_feed enforces the presence of a <feed> root.
        parse_feed(&xml, &self.config.feed_url).map(|_| ())
    }

    /// Crawl from `feed_url` to acquisition feeds and return the mapped books.
    ///
    /// The crawl is breadth-first and bounded by [`MAX_DEPTH`] navigation levels
    /// and [`MAX_PAGES`] total documents. A single unreachable page or malformed
    /// document is logged and skipped — it never aborts the whole scan.
    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        if self.config.feed_url.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut books = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        queue.push_back((self.config.feed_url.clone(), 0));
        let mut pages = 0u32;

        while let Some((url, depth)) = queue.pop_front() {
            if pages >= MAX_PAGES {
                eprintln!("libro: opds crawl hit page cap ({MAX_PAGES}), stopping");
                break;
            }
            if !visited.insert(url.clone()) {
                continue;
            }
            pages += 1;

            let xml = match self.fetch_text(&url).await {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("libro: opds skipping '{url}': {e}");
                    continue;
                }
            };
            let parsed = match parse_feed(&xml, &url) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("libro: opds bad feed '{url}': {e}");
                    continue;
                }
            };

            books.extend(parsed.books);

            // Pagination stays at the same depth.
            if let Some(next) = parsed.next {
                queue.push_back((next, depth));
            }
            // Descend into sub-catalogs until the depth cap.
            if depth < MAX_DEPTH {
                for nav in parsed.nav_hrefs {
                    queue.push_back((nav, depth + 1));
                }
            }
        }

        Ok(books)
    }
}

// ---------------------------------------------------------------------------
// Parsing (pure, network-free, unit-tested)
// ---------------------------------------------------------------------------

/// Parse one OPDS Atom feed document.
///
/// `base` is the URL the document was fetched from; relative links in the feed
/// are resolved to absolute URLs against it.
pub fn parse_feed(xml: &str, base: &str) -> ProviderResult<ParsedFeed> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| ProviderError::Api(format!("invalid OPDS XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "feed" {
        return Err(ProviderError::Api(
            "not an OPDS Atom feed (missing <feed> root)".into(),
        ));
    }

    let mut feed = ParsedFeed::default();

    // Feed-level pagination link (direct children only, so entry links are excluded).
    for link in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "link")
    {
        if link.attribute("rel") == Some("next") {
            if let Some(href) = link.attribute("href") {
                feed.next = Some(resolve_url(base, href));
            }
        }
    }

    for entry in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "entry")
    {
        match parse_entry(&entry, base) {
            EntryKind::Book(book) => feed.books.push(book),
            EntryKind::Nav(href) => feed.nav_hrefs.push(href),
            EntryKind::Skip => {}
        }
    }

    Ok(feed)
}

enum EntryKind {
    Book(Book),
    Nav(String),
    Skip,
}

fn parse_entry(entry: &roxmltree::Node, base: &str) -> EntryKind {
    let mut acquisitions: Vec<Acquisition> = Vec::new();
    let mut image: Option<String> = None;
    let mut thumbnail: Option<String> = None;
    let mut nav_href: Option<String> = None;

    for link in entry
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "link")
    {
        let href = match link.attribute("href") {
            Some(h) if !h.trim().is_empty() => h,
            _ => continue,
        };
        let rel = link.attribute("rel").unwrap_or("");
        let mime = link.attribute("type");
        let abs = resolve_url(base, href);

        if rel.starts_with(REL_ACQUISITION_PREFIX) {
            acquisitions.push(Acquisition {
                href: abs,
                mime: mime.map(str::to_string),
                rel: rel.to_string(),
            });
        } else if rel == REL_IMAGE {
            image.get_or_insert(abs);
        } else if rel == REL_THUMBNAIL {
            thumbnail.get_or_insert(abs);
        } else if rel == "subsection"
            || mime
                .map(|t| t.contains("application/atom+xml") && t.contains("opds-catalog"))
                .unwrap_or(false)
        {
            nav_href.get_or_insert(abs);
        }
    }

    if !acquisitions.is_empty() {
        EntryKind::Book(build_book(entry, base, &acquisitions, image.or(thumbnail)))
    } else if let Some(href) = nav_href {
        EntryKind::Nav(href)
    } else {
        EntryKind::Skip
    }
}

fn build_book(
    entry: &roxmltree::Node,
    _base: &str,
    acquisitions: &[Acquisition],
    cover: Option<String>,
) -> Book {
    let title = child_text(entry, "title").unwrap_or_else(|| "Untitled".to_string());

    // Prefer an open-access link, then an EPUB, then whatever came first.
    let primary = acquisitions
        .iter()
        .find(|a| a.rel.contains("open-access"))
        .or_else(|| {
            acquisitions
                .iter()
                .find(|a| a.mime.as_deref().map(|m| m.contains("epub")).unwrap_or(false))
        })
        .unwrap_or(&acquisitions[0]);

    let id = child_text(entry, "id")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| primary.href.clone());

    let media_type = if acquisitions
        .iter()
        .any(|a| a.mime.as_deref().map(|m| m.contains("audio")).unwrap_or(false))
    {
        MediaType::Audiobook
    } else {
        MediaType::Ebook
    };

    let mut book = Book::new(id, title, media_type, OpdsProvider::ID);

    // Authors: every <author><name>.
    for author in entry
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "author")
    {
        if let Some(name) = child_text(&author, "name") {
            if !name.is_empty() {
                book.authors.push(name);
            }
        }
    }

    // Description: <summary> preferred, else <content>.
    book.description = child_text(entry, "summary")
        .filter(|s| !s.is_empty())
        .or_else(|| child_text(entry, "content").filter(|s| !s.is_empty()));

    // Series: best-effort. Calibre-Web exposes it inconsistently; try a
    // <series> element or a <category> whose scheme mentions "series".
    book.series = extract_series(entry);

    // Identifiers: dc:identifier / dcterms:identifier (local name "identifier").
    let mut extra_id = 0;
    for ident in entry
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "identifier")
    {
        let raw = ident.text().unwrap_or("").trim().to_string();
        if raw.is_empty() {
            continue;
        }
        let digits = extract_isbn_digits(&raw);
        if is_isbn_shaped(&digits) {
            match digits.len() {
                13 => {
                    book.identifiers.entry("isbn13".into()).or_insert(digits);
                }
                10 => {
                    book.identifiers.entry("isbn10".into()).or_insert(digits);
                }
                _ => {}
            }
        } else {
            let key = format!("identifier{}", if extra_id == 0 { String::new() } else { extra_id.to_string() });
            book.identifiers.entry(key).or_insert(raw);
            extra_id += 1;
        }
    }

    if let Some(cover_url) = cover {
        book.cover_url = Some(cover_url);
    }

    // Carry the primary acquisition link forward so the frontend / download_book
    // can fetch the file. (Skeleton shortcut: stored in identifiers; a later
    // phase adds a dedicated `acquisitions` field to Book — see TODO.)
    book.identifiers
        .insert("opds:acquisition_url".into(), primary.href.clone());
    if let Some(mime) = &primary.mime {
        book.identifiers
            .insert("opds:acquisition_type".into(), mime.clone());
    }

    book
}

fn extract_series(entry: &roxmltree::Node) -> Option<String> {
    for node in entry.children().filter(|n| n.is_element()) {
        if node.tag_name().name() == "series" {
            let t = node.text().unwrap_or("").trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
        if node.tag_name().name() == "category" {
            let is_series = node
                .attribute("scheme")
                .map(|s| s.to_ascii_lowercase().contains("series"))
                .unwrap_or(false);
            if is_series {
                if let Some(v) = node.attribute("label").or_else(|| node.attribute("term")) {
                    if !v.trim().is_empty() {
                        return Some(v.trim().to_string());
                    }
                }
            }
        }
    }
    None
}

/// First direct child element with the given local name, trimmed text.
fn child_text(node: &roxmltree::Node, name: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .map(|n| n.text().unwrap_or("").trim().to_string())
}

fn extract_isbn_digits(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let body = lower
        .strip_prefix("urn:isbn:")
        .or_else(|| lower.strip_prefix("isbn:"))
        .unwrap_or(&lower);
    body.chars()
        .filter(|c| c.is_ascii_digit() || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn is_isbn_shaped(digits: &str) -> bool {
    (digits.len() == 13 && digits.chars().all(|c| c.is_ascii_digit())) || digits.len() == 10
}

// ---------------------------------------------------------------------------
// URL resolution (relative -> absolute against the feed URL)
// ---------------------------------------------------------------------------

fn has_scheme(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:")
}

/// Resolve `href` (possibly relative) against the `base` feed URL.
pub fn resolve_url(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.is_empty() {
        return base.to_string();
    }
    if has_scheme(href) {
        return href.to_string();
    }
    let (scheme, rest) = match base.split_once("://") {
        Some(pair) => pair,
        None => return href.to_string(),
    };
    // Scheme-relative: //authority/path
    if let Some(after) = href.strip_prefix("//") {
        return format!("{scheme}://{}", normalize(after));
    }
    let (authority, base_path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let full_path = if href.starts_with('/') {
        href.to_string()
    } else {
        // Strip any query/fragment from the base path before taking its dir.
        let path_only = base_path
            .split(['?', '#'])
            .next()
            .unwrap_or(&base_path);
        let dir = path_only.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        format!("{dir}/{href}")
    };
    format!("{scheme}://{authority}{}", normalize(&full_path))
}

/// Collapse `.`/`..` path segments, preserving a leading `/` and any
/// query/fragment suffix.
fn normalize(path_and_query: &str) -> String {
    let (path, suffix) = match path_and_query.find(['?', '#']) {
        Some(i) => (&path_and_query[..i], &path_and_query[i..]),
        None => (path_and_query, ""),
    };
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut segs: Vec<&str> = Vec::new();
    for s in path.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            other => segs.push(other),
        }
    }
    let mut out = String::new();
    if absolute {
        out.push('/');
    }
    out.push_str(&segs.join("/"));
    // Preserve a meaningful trailing slash (some servers route on it).
    if trailing && !segs.is_empty() && !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(suffix);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAV_FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Library</title>
  <link rel="self" href="/opds" type="application/atom+xml;profile=opds-catalog;kind=navigation"/>
  <entry>
    <title>All Books</title>
    <link rel="subsection" href="/opds/books" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  </entry>
  <entry>
    <title>By Author</title>
    <link href="authors" type="application/atom+xml;profile=opds-catalog;kind=navigation"/>
  </entry>
</feed>"#;

    const ACQ_FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"
      xmlns:dcterms="http://purl.org/dc/terms/">
  <title>All Books</title>
  <link rel="self" href="/opds/books?page=1" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <link rel="next" href="/opds/books?page=2" type="application/atom+xml;profile=opds-catalog;kind=acquisition"/>
  <entry>
    <title>Effective Java</title>
    <id>urn:uuid:aaaa</id>
    <author><name>Joshua Bloch</name></author>
    <summary>The definitive guide.</summary>
    <dcterms:identifier>urn:isbn:9780134685991</dcterms:identifier>
    <link rel="http://opds-spec.org/image" href="../covers/1.jpg" type="image/jpeg"/>
    <link rel="http://opds-spec.org/image/thumbnail" href="../covers/1t.jpg" type="image/jpeg"/>
    <link rel="http://opds-spec.org/acquisition" href="../download/1.epub" type="application/epub+zip"/>
  </entry>
  <entry>
    <title>The Odyssey</title>
    <author><name>Homer</name></author>
    <content type="text">An epic poem.</content>
    <link rel="http://opds-spec.org/acquisition/open-access" href="/download/2.epub" type="application/epub+zip"/>
    <link rel="http://opds-spec.org/acquisition" href="/download/2.m4b" type="audio/mp4"/>
  </entry>
</feed>"#;

    const BASE: &str = "https://example.org/opds/books?page=1";

    #[test]
    fn navigation_feed_yields_nav_targets_not_books() {
        let feed = parse_feed(NAV_FEED, "https://example.org/opds").unwrap();
        assert!(!feed.is_acquisition());
        assert!(feed.books.is_empty());
        assert_eq!(feed.nav_hrefs.len(), 2);
        // Root-relative and path-relative both resolved to absolute.
        assert_eq!(feed.nav_hrefs[0], "https://example.org/opds/books");
        assert_eq!(feed.nav_hrefs[1], "https://example.org/authors");
        assert!(feed.next.is_none());
    }

    #[test]
    fn acquisition_feed_maps_entries_to_books() {
        let feed = parse_feed(ACQ_FEED, BASE).unwrap();
        assert!(feed.is_acquisition());
        assert_eq!(feed.books.len(), 2);

        let ej = &feed.books[0];
        assert_eq!(ej.title, "Effective Java");
        assert_eq!(ej.authors, vec!["Joshua Bloch"]);
        assert_eq!(ej.description.as_deref(), Some("The definitive guide."));
        assert_eq!(ej.media_type, MediaType::Ebook);
        assert_eq!(ej.source_provider_id, "opds");
        // ISBN detected from urn:isbn.
        assert_eq!(ej.identifiers.get("isbn13").map(String::as_str), Some("9780134685991"));
        // Relative cover resolved to absolute (prefers full image over thumbnail).
        assert_eq!(
            ej.identifiers.get("opds:acquisition_url").map(String::as_str),
            Some("https://example.org/download/1.epub")
        );
        assert_eq!(ej.cover_url.as_deref(), Some("https://example.org/covers/1.jpg"));
        assert_eq!(
            ej.identifiers.get("opds:acquisition_type").map(String::as_str),
            Some("application/epub+zip")
        );
    }

    #[test]
    fn open_access_and_audio_links_are_classified() {
        let feed = parse_feed(ACQ_FEED, BASE).unwrap();
        let odyssey = &feed.books[1];
        assert_eq!(odyssey.title, "The Odyssey");
        // Content used as description fallback.
        assert_eq!(odyssey.description.as_deref(), Some("An epic poem."));
        // An audio acquisition link flips media_type to Audiobook.
        assert_eq!(odyssey.media_type, MediaType::Audiobook);
        // Primary link prefers the open-access acquisition.
        assert_eq!(
            odyssey.identifiers.get("opds:acquisition_url").map(String::as_str),
            Some("https://example.org/download/2.epub")
        );
    }

    #[test]
    fn detects_next_pagination_link_absolute() {
        let feed = parse_feed(ACQ_FEED, BASE).unwrap();
        assert_eq!(feed.next.as_deref(), Some("https://example.org/opds/books?page=2"));
    }

    #[test]
    fn malformed_entry_is_skipped_not_fatal() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <entry><title>No links here</title></entry>
            <entry>
              <title>Real Book</title>
              <link rel="http://opds-spec.org/acquisition" href="/b.epub" type="application/epub+zip"/>
            </entry>
        </feed>"#;
        let feed = parse_feed(xml, "https://x.test/opds").unwrap();
        assert_eq!(feed.books.len(), 1);
        assert_eq!(feed.books[0].title, "Real Book");
    }

    #[test]
    fn non_feed_xml_is_an_error() {
        assert!(parse_feed("<html><body>nope</body></html>", "https://x.test").is_err());
        assert!(parse_feed("this is not xml at <all", "https://x.test").is_err());
    }

    #[test]
    fn resolve_url_handles_all_forms() {
        let base = "https://ex.org/opds/books?page=1";
        assert_eq!(resolve_url(base, "https://other/x.jpg"), "https://other/x.jpg");
        assert_eq!(resolve_url(base, "/download/1.epub"), "https://ex.org/download/1.epub");
        assert_eq!(resolve_url(base, "cover.jpg"), "https://ex.org/opds/cover.jpg");
        assert_eq!(resolve_url(base, "../covers/1.jpg"), "https://ex.org/covers/1.jpg");
        assert_eq!(resolve_url(base, "//cdn.ex.org/a.jpg"), "https://cdn.ex.org/a.jpg");
        assert_eq!(resolve_url("https://ex.org", "/opds"), "https://ex.org/opds");
    }

    #[tokio::test]
    async fn empty_feed_url_lists_empty_without_network() {
        let provider = OpdsProvider::new(OpdsConfig::default());
        assert!(provider.list_library().await.unwrap().is_empty());
    }
}
