//! Google Books metadata provider (official public API).
//!
//! The [Google Books API](https://developers.google.com/books/docs/v1/using)
//! works anonymously; an optional API key (from config) raises the rate limit.
//! Libro only ever reads public volume metadata.
//!
//! Endpoints used:
//! * ISBN   — `GET /books/v1/volumes?q=isbn:{isbn}`
//! * Search — `GET /books/v1/volumes?q={query}&maxResults={n}`
//!
//! A `&country=` param is sent because Google requires it in some regions, and
//! `&key={api_key}` is appended when configured.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeMap;

use super::{BookMetadata, MetadataError, MetadataProvider, MetadataResult, USER_AGENT};

const BASE: &str = "https://www.googleapis.com/books/v1/volumes";

/// The Google Books metadata provider.
pub struct GoogleBooksProvider {
    client: reqwest::Client,
    api_key: Option<String>,
}

impl GoogleBooksProvider {
    pub const ID: &'static str = "googlebooks";

    /// Create a provider, optionally with an API key to raise rate limits.
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
        }
    }

    /// Build a `/volumes` URL for the given `q` value plus common params.
    fn volumes_url(&self, q: &str, max_results: Option<usize>) -> String {
        let mut url = format!("{BASE}?q={}&country=US", urlencode(q));
        if let Some(n) = max_results {
            url.push_str(&format!("&maxResults={}", n.clamp(1, 40)));
        }
        if let Some(key) = &self.api_key {
            url.push_str(&format!("&key={}", urlencode(key)));
        }
        url
    }

    async fn get_volumes(&self, url: &str) -> MetadataResult<GbVolumes> {
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    MetadataError::Network("timeout calling Google Books".into())
                } else {
                    MetadataError::Network(e.to_string())
                }
            })?;

        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| MetadataError::Network(e.to_string()))?;

        // Google returns errors as `{ "error": { code, message, ... } }`.
        if let Ok(err) = serde_json::from_str::<GbErrorEnvelope>(&text) {
            let msg = err.error.message;
            return Err(MetadataError::Api(format!("google books {status}: {msg}")));
        }
        if status != 200 {
            return Err(MetadataError::Api(format!("unexpected status {status}")));
        }
        serde_json::from_str::<GbVolumes>(&text)
            .map_err(|e| MetadataError::Other(format!("invalid JSON from Google Books: {e}")))
    }
}

#[async_trait]
impl MetadataProvider for GoogleBooksProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    async fn by_isbn(&self, isbn: &str) -> MetadataResult<Option<BookMetadata>> {
        let isbn = isbn.trim();
        if isbn.is_empty() {
            return Ok(None);
        }
        let url = self.volumes_url(&format!("isbn:{isbn}"), Some(1));
        let volumes = self.get_volumes(&url).await?;
        Ok(volumes.items.into_iter().next().map(map_gb_volume))
    }

    async fn search(&self, query: &str, limit: usize) -> MetadataResult<Vec<BookMetadata>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.volumes_url(query, Some(limit));
        let volumes = self.get_volumes(&url).await?;
        Ok(volumes.items.into_iter().map(map_gb_volume).collect())
    }
}

// ---------------------------------------------------------------------------
// Google Books response types (only the fields Libro needs).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct GbVolumes {
    #[serde(default)]
    items: Vec<GbVolume>,
}

#[derive(Debug, Deserialize)]
struct GbVolume {
    #[serde(default)]
    id: String,
    #[serde(default)]
    #[serde(rename = "volumeInfo")]
    volume_info: GbVolumeInfo,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GbVolumeInfo {
    #[serde(default)]
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    publisher: Option<String>,
    #[serde(default)]
    published_date: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    industry_identifiers: Vec<GbIndustryId>,
    #[serde(default)]
    page_count: Option<u32>,
    #[serde(default)]
    image_links: Option<GbImageLinks>,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GbIndustryId {
    #[serde(default, rename = "type")]
    id_type: String,
    #[serde(default)]
    identifier: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GbImageLinks {
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    small_thumbnail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GbErrorEnvelope {
    error: GbError,
}

#[derive(Debug, Deserialize)]
struct GbError {
    #[serde(default)]
    message: String,
}

// ---------------------------------------------------------------------------
// Pure mapping helpers (unit-tested without any network access).
// ---------------------------------------------------------------------------

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

/// Prefer https for cover URLs (Google often returns `http` thumbnails).
fn https(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        url.to_string()
    }
}

/// Map a Google Books volume to normalized [`BookMetadata`].
fn map_gb_volume(v: GbVolume) -> BookMetadata {
    let info = v.volume_info;

    let mut identifiers = BTreeMap::new();
    for id in &info.industry_identifiers {
        match id.id_type.as_str() {
            "ISBN_13" => {
                identifiers.insert("isbn13".to_string(), id.identifier.clone());
            }
            "ISBN_10" => {
                identifiers.insert("isbn10".to_string(), id.identifier.clone());
            }
            _ => {}
        }
    }
    if !v.id.is_empty() {
        identifiers.insert("google_volume_id".to_string(), v.id);
    }

    let cover_url = info.image_links.as_ref().and_then(|l| {
        l.thumbnail
            .clone()
            .or_else(|| l.small_thumbnail.clone())
            .map(|u| https(&u))
    });

    BookMetadata {
        title: info.title,
        subtitle: info.subtitle.filter(|s| !s.is_empty()),
        authors: info.authors,
        description: info.description.filter(|s| !s.is_empty()),
        cover_url,
        series: None,
        identifiers,
        publish_date: info.published_date.filter(|s| !s.is_empty()),
        page_count: info.page_count,
        publisher: info.publisher.filter(|s| !s.is_empty()),
        language: info.language.filter(|s| !s.is_empty()),
        source: GoogleBooksProvider::ID.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shaped after a live `?q=isbn:9780134685991` response (volumeInfo trimmed
    // to the mapped fields).
    fn isbn_fixture() -> &'static str {
        r#"{
          "kind": "books#volumes",
          "totalItems": 1,
          "items": [
            {
              "id": "BIpDDwAAQBAJ",
              "volumeInfo": {
                "title": "Effective Java",
                "authors": ["Joshua Bloch"],
                "publisher": "Addison-Wesley Professional",
                "publishedDate": "2017-12-18",
                "description": "The definitive guide to Java best practices.",
                "industryIdentifiers": [
                  {"type": "ISBN_13", "identifier": "9780134685991"},
                  {"type": "ISBN_10", "identifier": "0134685997"}
                ],
                "pageCount": 416,
                "imageLinks": {
                  "smallThumbnail": "http://books.google.com/books/content?id=BIpDDwAAQBAJ&img=1&zoom=5",
                  "thumbnail": "http://books.google.com/books/content?id=BIpDDwAAQBAJ&img=1&zoom=1"
                },
                "language": "en"
              }
            }
          ]
        }"#
    }

    fn empty_fixture() -> &'static str {
        r#"{ "kind": "books#volumes", "totalItems": 0 }"#
    }

    fn error_fixture() -> &'static str {
        r#"{ "error": { "code": 429, "message": "Quota exceeded", "status": "RESOURCE_EXHAUSTED" } }"#
    }

    #[test]
    fn parses_isbn_volume_into_metadata() {
        let vols: GbVolumes = serde_json::from_str(isbn_fixture()).unwrap();
        let meta = map_gb_volume(vols.items.into_iter().next().unwrap());

        assert_eq!(meta.title, "Effective Java");
        assert_eq!(meta.authors, vec!["Joshua Bloch"]);
        assert_eq!(meta.page_count, Some(416));
        assert_eq!(meta.publisher.as_deref(), Some("Addison-Wesley Professional"));
        assert_eq!(meta.language.as_deref(), Some("en"));
        assert_eq!(meta.description.as_deref(), Some("The definitive guide to Java best practices."));
        assert_eq!(meta.identifiers.get("isbn13").map(String::as_str), Some("9780134685991"));
        assert_eq!(meta.identifiers.get("isbn10").map(String::as_str), Some("0134685997"));
        assert_eq!(meta.identifiers.get("google_volume_id").map(String::as_str), Some("BIpDDwAAQBAJ"));
        // Cover upgraded to https.
        assert!(meta.cover_url.as_deref().unwrap().starts_with("https://"));
        assert_eq!(meta.source, "googlebooks");
    }

    #[test]
    fn empty_volumes_yields_no_items() {
        let vols: GbVolumes = serde_json::from_str(empty_fixture()).unwrap();
        assert!(vols.items.is_empty());
    }

    #[test]
    fn error_envelope_is_detected() {
        assert!(serde_json::from_str::<GbErrorEnvelope>(error_fixture()).is_ok());
        // A normal volumes payload must NOT parse as an error envelope.
        assert!(serde_json::from_str::<GbErrorEnvelope>(empty_fixture()).is_err());
    }

    #[test]
    fn https_upgrades_http_urls() {
        assert_eq!(https("http://x/y"), "https://x/y");
        assert_eq!(https("https://x/y"), "https://x/y");
    }
}
