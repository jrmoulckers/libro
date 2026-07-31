//! Open Library metadata provider (official public API, no auth).
//!
//! Open Library (<https://openlibrary.org>) is a project of the Internet Archive
//! with a free, keyless API. Libro identifies itself via a descriptive
//! `User-Agent` as the API docs request.
//!
//! Endpoints used:
//! * ISBN   — `GET /api/books?bibkeys=ISBN:{isbn}&format=json&jscmd=data`
//!   (<https://openlibrary.org/dev/docs/api/books>)
//! * Search — `GET /search.json?q={query}&limit={n}&fields=...`
//!   (<https://openlibrary.org/dev/docs/api/search>)
//! * Covers — `https://covers.openlibrary.org/b/id/{cover_id}-L.jpg`
//!   (<https://openlibrary.org/dev/docs/api/covers>)

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::BTreeMap;

use super::{BookMetadata, MetadataError, MetadataProvider, MetadataResult, USER_AGENT};

const BASE: &str = "https://openlibrary.org";
const COVERS_BASE: &str = "https://covers.openlibrary.org";

/// The Open Library metadata provider.
pub struct OpenLibraryProvider {
    client: reqwest::Client,
}

impl Default for OpenLibraryProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenLibraryProvider {
    pub const ID: &'static str = "openlibrary";

    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// GET `url` and decode JSON as `T`, mapping transport/status errors.
    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> MetadataResult<T> {
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    MetadataError::Network(format!("timeout: {url}"))
                } else {
                    MetadataError::Network(e.to_string())
                }
            })?;

        match resp.status().as_u16() {
            200 => {
                let text = resp
                    .text()
                    .await
                    .map_err(|e| MetadataError::Network(e.to_string()))?;
                serde_json::from_str::<T>(&text)
                    .map_err(|e| MetadataError::Other(format!("invalid JSON from {url}: {e}")))
            }
            429 => Err(MetadataError::Api("rate limited by Open Library".into())),
            other => Err(MetadataError::Api(format!("unexpected status {other} from {url}"))),
        }
    }

    /// Best-effort description lookup for an edition OLID (e.g. `OL…M`).
    ///
    /// Open Library's `jscmd=data` books payload omits descriptions, so we follow
    /// the edition doc (`/books/{olid}.json`) and, if it has no description, the
    /// linked work (`/works/{id}.json`). Any failure returns `None` — enrichment
    /// treats a missing description as simply absent, never an error.
    async fn fetch_edition_description(&self, edition_olid: &str) -> Option<String> {
        let edition_url = format!("{BASE}/books/{edition_olid}.json");
        let edition: OlDoc = self.get_json(&edition_url).await.ok()?;
        if let Some(desc) = edition.description.and_then(desc_to_string) {
            return Some(desc);
        }
        // Fall back to the work record, following the first work key.
        let work_key = edition.works.into_iter().next()?.key;
        if work_key.is_empty() {
            return None;
        }
        let work_url = format!("{BASE}{work_key}.json");
        let work: OlDoc = self.get_json(&work_url).await.ok()?;
        work.description.and_then(desc_to_string)
    }
}

#[async_trait]
impl MetadataProvider for OpenLibraryProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    async fn by_isbn(&self, isbn: &str) -> MetadataResult<Option<BookMetadata>> {
        let isbn = isbn.trim();
        if isbn.is_empty() {
            return Ok(None);
        }
        let bibkey = format!("ISBN:{isbn}");
        let url = format!(
            "{BASE}/api/books?bibkeys={}&format=json&jscmd=data",
            urlencode(&bibkey)
        );
        // The Books API returns a map keyed by the bibkey; an unknown ISBN yields
        // an empty object (not a 404).
        let map: BTreeMap<String, OlBook> = self.get_json(&url).await?;
        let Some(ol) = map.into_values().next() else {
            return Ok(None);
        };
        let mut meta = map_ol_book(ol);
        // The `jscmd=data` payload has no description, so best-effort fetch it
        // from the edition/works endpoints (a failure just leaves it `None`).
        if meta.description.is_none() {
            if let Some(olid) = meta.identifiers.get("olid").cloned() {
                meta.description = self.fetch_edition_description(&olid).await;
            }
        }
        Ok(Some(meta))
    }

    async fn search(&self, query: &str, limit: usize) -> MetadataResult<Vec<BookMetadata>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 50);
        let url = format!(
            "{BASE}/search.json?q={}&limit={limit}&fields=key,title,subtitle,author_name,first_publish_year,cover_i,cover_edition_key,isbn,language,number_of_pages_median,publisher",
            urlencode(query)
        );
        let resp: OlSearchResponse = self.get_json(&url).await?;
        Ok(resp.docs.iter().map(map_ol_search_doc).collect())
    }
}

// ---------------------------------------------------------------------------
// Open Library response types (only the fields Libro needs).
// ---------------------------------------------------------------------------

/// A value in the `/api/books` (`jscmd=data`) response map.
#[derive(Debug, Default, Deserialize)]
struct OlBook {
    #[serde(default)]
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    authors: Vec<OlNamed>,
    #[serde(default)]
    publishers: Vec<OlNamed>,
    #[serde(default)]
    publish_date: Option<String>,
    #[serde(default)]
    number_of_pages: Option<u32>,
    #[serde(default)]
    identifiers: OlIdentifiers,
    #[serde(default)]
    cover: Option<OlCover>,
}

#[derive(Debug, Default, Deserialize)]
struct OlNamed {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct OlIdentifiers {
    #[serde(default)]
    isbn_10: Vec<String>,
    #[serde(default)]
    isbn_13: Vec<String>,
    #[serde(default)]
    openlibrary: Vec<String>,
    #[serde(default)]
    goodreads: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OlCover {
    #[serde(default)]
    large: Option<String>,
    #[serde(default)]
    medium: Option<String>,
    #[serde(default)]
    small: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OlSearchResponse {
    #[serde(default)]
    docs: Vec<OlSearchDoc>,
}

/// Shared shape for the edition (`/books/{olid}.json`) and work
/// (`/works/{id}.json`) documents — we only need description + work links.
#[derive(Debug, Default, Deserialize)]
struct OlDoc {
    #[serde(default)]
    description: Option<OlDescription>,
    #[serde(default)]
    works: Vec<OlKeyRef>,
}

#[derive(Debug, Deserialize)]
struct OlKeyRef {
    #[serde(default)]
    key: String,
}

/// Open Library returns descriptions either as a plain string or as a typed
/// `{ "type": "/type/text", "value": "…" }` object.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OlDescription {
    Text(String),
    Typed { value: String },
}

/// Normalize an [`OlDescription`] to a trimmed, non-empty `String`.
fn desc_to_string(desc: OlDescription) -> Option<String> {
    let value = match desc {
        OlDescription::Text(s) => s,
        OlDescription::Typed { value } => value,
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Debug, Default, Deserialize)]
struct OlSearchDoc {
    #[serde(default)]
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    author_name: Vec<String>,
    #[serde(default)]
    first_publish_year: Option<i32>,
    #[serde(default)]
    cover_i: Option<i64>,
    #[serde(default)]
    cover_edition_key: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    isbn: Vec<String>,
    #[serde(default)]
    language: Vec<String>,
    #[serde(default)]
    number_of_pages_median: Option<u32>,
    #[serde(default)]
    publisher: Vec<String>,
}

// ---------------------------------------------------------------------------
// Pure mapping helpers (unit-tested without any network access).
// ---------------------------------------------------------------------------

/// Minimal `application/x-www-form-urlencoded`-style escaping for query values.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn first_nonempty(list: &[String]) -> Option<String> {
    list.iter().find(|s| !s.trim().is_empty()).cloned()
}

/// Map an `/api/books` record to normalized [`BookMetadata`].
fn map_ol_book(b: OlBook) -> BookMetadata {
    let mut identifiers = BTreeMap::new();
    if let Some(v) = first_nonempty(&b.identifiers.isbn_13) {
        identifiers.insert("isbn13".to_string(), v);
    }
    if let Some(v) = first_nonempty(&b.identifiers.isbn_10) {
        identifiers.insert("isbn10".to_string(), v);
    }
    if let Some(v) = first_nonempty(&b.identifiers.openlibrary) {
        identifiers.insert("olid".to_string(), v);
    }
    if let Some(v) = first_nonempty(&b.identifiers.goodreads) {
        identifiers.insert("goodreads".to_string(), v);
    }

    let cover_url = b.cover.as_ref().and_then(|c| {
        c.large
            .clone()
            .or_else(|| c.medium.clone())
            .or_else(|| c.small.clone())
    });

    BookMetadata {
        title: b.title,
        subtitle: b.subtitle.filter(|s| !s.is_empty()),
        authors: b.authors.into_iter().map(|a| a.name).filter(|n| !n.is_empty()).collect(),
        description: None, // jscmd=data does not include a description.
        cover_url,
        series: None,
        identifiers,
        publish_date: b.publish_date.filter(|s| !s.is_empty()),
        page_count: b.number_of_pages,
        publisher: b.publishers.into_iter().map(|p| p.name).find(|n| !n.is_empty()),
        language: None,
        source: OpenLibraryProvider::ID.to_string(),
    }
}

/// Map a `/search.json` doc to normalized [`BookMetadata`].
fn map_ol_search_doc(d: &OlSearchDoc) -> BookMetadata {
    let mut identifiers = BTreeMap::new();
    // Prefer a 13-digit ISBN, fall back to a 10-digit one.
    if let Some(isbn13) = d.isbn.iter().find(|s| s.len() == 13) {
        identifiers.insert("isbn13".to_string(), isbn13.clone());
    }
    if let Some(isbn10) = d.isbn.iter().find(|s| s.len() == 10) {
        identifiers.insert("isbn10".to_string(), isbn10.clone());
    }
    // The work key (`/works/OL...W`) doubles as a stable Open Library id.
    if let Some(key) = &d.key {
        identifiers.insert(
            "olid".to_string(),
            key.trim_start_matches("/works/").to_string(),
        );
    }

    // Covers: prefer the numeric cover id, else the cover edition OLID.
    let cover_url = d
        .cover_i
        .map(|id| format!("{COVERS_BASE}/b/id/{id}-L.jpg"))
        .or_else(|| {
            d.cover_edition_key
                .as_ref()
                .map(|olid| format!("{COVERS_BASE}/b/olid/{olid}-L.jpg"))
        });

    BookMetadata {
        title: d.title.clone(),
        subtitle: d.subtitle.clone().filter(|s| !s.is_empty()),
        authors: d.author_name.clone(),
        description: None,
        cover_url,
        series: None,
        identifiers,
        publish_date: d.first_publish_year.map(|y| y.to_string()),
        page_count: d.number_of_pages_median,
        publisher: first_nonempty(&d.publisher),
        language: first_nonempty(&d.language),
        source: OpenLibraryProvider::ID.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from the live `/api/books?...jscmd=data` endpoint for
    // ISBN 9780134685991 (Effective Java), trimmed to the mapped fields.
    fn books_fixture() -> &'static str {
        r#"{
          "ISBN:9780134685991": {
            "title": "Effective Java",
            "authors": [{"url": "https://openlibrary.org/authors/OL1607920A/Joshua_Bloch", "name": "Joshua Bloch"}],
            "number_of_pages": 416,
            "identifiers": {
              "isbn_10": ["0134685997"],
              "isbn_13": ["9780134685991"],
              "openlibrary": ["OL31838212M"]
            },
            "publishers": [{"name": "Addison-Wesley Professional"}],
            "publish_date": "December 27, 2017",
            "cover": {
              "small": "https://covers.openlibrary.org/b/id/12420356-S.jpg",
              "medium": "https://covers.openlibrary.org/b/id/12420356-M.jpg",
              "large": "https://covers.openlibrary.org/b/id/12420356-L.jpg"
            }
          }
        }"#
    }

    fn search_fixture() -> &'static str {
        r#"{
          "numFound": 5,
          "docs": [
            {
              "author_name": ["Joshua Bloch"],
              "cover_edition_key": "OL9653361M",
              "cover_i": 1176573,
              "first_publish_year": 2001,
              "isbn": ["0201310058", "9780201310054"],
              "key": "/works/OL6223299W",
              "language": ["eng", "chi"],
              "title": "Effective Java"
            }
          ]
        }"#
    }

    #[test]
    fn parses_isbn_lookup_into_metadata() {
        let map: BTreeMap<String, OlBook> = serde_json::from_str(books_fixture()).unwrap();
        let meta = map_ol_book(map.into_values().next().unwrap());

        assert_eq!(meta.title, "Effective Java");
        assert_eq!(meta.authors, vec!["Joshua Bloch"]);
        assert_eq!(meta.page_count, Some(416));
        assert_eq!(meta.publisher.as_deref(), Some("Addison-Wesley Professional"));
        assert_eq!(meta.publish_date.as_deref(), Some("December 27, 2017"));
        assert_eq!(meta.identifiers.get("isbn13").map(String::as_str), Some("9780134685991"));
        assert_eq!(meta.identifiers.get("isbn10").map(String::as_str), Some("0134685997"));
        assert_eq!(meta.identifiers.get("olid").map(String::as_str), Some("OL31838212M"));
        assert_eq!(
            meta.cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/12420356-L.jpg")
        );
        assert_eq!(meta.source, "openlibrary");
    }

    #[test]
    fn parses_search_docs_into_metadata() {
        let resp: OlSearchResponse = serde_json::from_str(search_fixture()).unwrap();
        let results: Vec<BookMetadata> = resp.docs.iter().map(map_ol_search_doc).collect();

        assert_eq!(results.len(), 1);
        let m = &results[0];
        assert_eq!(m.title, "Effective Java");
        assert_eq!(m.authors, vec!["Joshua Bloch"]);
        assert_eq!(m.publish_date.as_deref(), Some("2001"));
        assert_eq!(m.language.as_deref(), Some("eng"));
        assert_eq!(m.identifiers.get("isbn13").map(String::as_str), Some("9780201310054"));
        assert_eq!(m.identifiers.get("isbn10").map(String::as_str), Some("0201310058"));
        assert_eq!(m.identifiers.get("olid").map(String::as_str), Some("OL6223299W"));
        assert_eq!(
            m.cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/1176573-L.jpg")
        );
    }

    #[test]
    fn empty_books_map_means_not_found() {
        let map: BTreeMap<String, OlBook> = serde_json::from_str("{}").unwrap();
        assert!(map.into_values().next().is_none());
    }

    #[test]
    fn urlencode_escapes_spaces_and_reserved() {
        assert_eq!(urlencode("effective java"), "effective+java");
        assert_eq!(urlencode("ISBN:123"), "ISBN%3A123");
    }

    #[test]
    fn parses_string_and_typed_descriptions() {
        // Work doc with a plain-string description.
        let work_str: OlDoc =
            serde_json::from_str(r#"{ "description": "A plain string synopsis." }"#).unwrap();
        assert_eq!(
            work_str.description.and_then(desc_to_string).as_deref(),
            Some("A plain string synopsis.")
        );

        // Edition doc with a typed description object + a work link.
        let edition_typed: OlDoc = serde_json::from_str(
            r#"{
              "description": { "type": "/type/text", "value": "  Typed synopsis.  " },
              "works": [{ "key": "/works/OL6223299W" }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            edition_typed.description.and_then(desc_to_string).as_deref(),
            Some("Typed synopsis.")
        );
        assert_eq!(edition_typed.works[0].key, "/works/OL6223299W");
    }

    #[test]
    fn empty_or_missing_description_is_none() {
        let no_desc: OlDoc = serde_json::from_str(r#"{ "works": [] }"#).unwrap();
        assert!(no_desc.description.and_then(desc_to_string).is_none());

        let blank: OlDoc = serde_json::from_str(r#"{ "description": "   " }"#).unwrap();
        assert!(blank.description.and_then(desc_to_string).is_none());
    }
}
