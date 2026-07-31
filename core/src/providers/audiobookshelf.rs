//! Audiobookshelf connector.
//!
//! [Audiobookshelf](https://www.audiobookshelf.org/) (ABS) is a **self-hosted**
//! audiobook/podcast/ebook server the user runs themselves. Because Libro only
//! ever talks to the user's *own* server over its official REST API, this is a
//! fully legitimate integration (see `ARCHITECTURE.md` → "Legal boundaries").
//!
//! Auth is a per-user API token sent as `Authorization: Bearer {token}`.
//!
//! API reference: <https://api.audiobookshelf.org/>
//!
//! Endpoints used:
//! * `GET /api/me`                     — validate the token, read media progress.
//! * `GET /api/libraries`              — enumerate libraries.
//! * `GET /api/libraries/{id}/items`   — list items in a library (minified).
//! * `GET /api/items/{id}/cover`       — per-item cover image.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::{Book, MediaType, Progress};
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

/// Connection settings for an Audiobookshelf instance.
///
/// Persisted (encrypted) via [`crate::config`]. The `api_token` is a long-lived
/// user API token from the ABS account settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudiobookshelfConfig {
    /// Base URL of the server, e.g. `https://abs.example.com`.
    pub base_url: String,
    /// API token for the authenticating user.
    pub api_token: String,
}

/// The Audiobookshelf connector.
pub struct AudiobookshelfProvider {
    config: AudiobookshelfConfig,
    authenticated: bool,
    client: reqwest::Client,
}

impl AudiobookshelfProvider {
    pub const ID: &'static str = "audiobookshelf";

    pub fn new(config: AudiobookshelfConfig) -> Self {
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
}

#[async_trait]
impl Provider for AudiobookshelfProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "Audiobookshelf"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // ABS exposes the library, per-item downloads, and progress sync.
        ProviderCapabilities::CATALOG
            | ProviderCapabilities::DOWNLOAD
            | ProviderCapabilities::PROGRESS_SYNC
    }

    async fn authenticate(&mut self, config: &serde_json::Value) -> ProviderResult<()> {
        // Allow (re)configuration from the stored settings blob.
        if !config.is_null() {
            self.config = serde_json::from_value(config.clone())
                .map_err(|e| ProviderError::Config(e.to_string()))?;
        }
        if self.base().is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }

        // Validate the token against `/api/me`.
        let url = format!("{}/api/me", self.base());
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => {
                // Parse to confirm it's really an ABS user payload.
                let _me: AbsMe = resp
                    .json()
                    .await
                    .map_err(|e| ProviderError::Other(format!("invalid /api/me response: {e}")))?;
                self.authenticated = true;
                Ok(())
            }
            401 | 403 => Err(ProviderError::NotAuthenticated),
            other => Err(ProviderError::Other(format!(
                "unexpected status {other} from {url}"
            ))),
        }
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        if !self.authenticated {
            return Err(ProviderError::NotAuthenticated);
        }
        let base = self.base().to_string();
        let token = self.config.api_token.clone();

        // 1. Enumerate libraries.
        let libs: AbsLibrariesResponse = self
            .client
            .get(format!("{base}/api/libraries"))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("invalid /api/libraries response: {e}")))?;

        // 2. Pull each library's items and map them.
        let mut books: Vec<Book> = Vec::new();
        for lib in &libs.libraries {
            let items: AbsLibraryItemsResponse = self
                .client
                .get(format!("{base}/api/libraries/{}/items", lib.id))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| ProviderError::Network(e.to_string()))?
                .json()
                .await
                .map_err(|e| {
                    ProviderError::Other(format!("invalid items response for {}: {e}", lib.id))
                })?;

            for item in &items.results {
                books.push(map_item_to_book(item, &lib.media_type, &base, &token, Self::ID));
            }
        }

        // 3. Merge listening/reading progress from `/api/me` (best-effort).
        match self
            .client
            .get(format!("{base}/api/me"))
            .bearer_auth(&token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(resp) => {
                if let Ok(me) = resp.json::<AbsMe>().await {
                    merge_progress(&mut books, &me.media_progress);
                }
            }
            // Progress is best-effort; a failure here shouldn't drop the catalog.
            Err(e) => eprintln!("libro: audiobookshelf progress fetch failed: {e}"),
        }

        Ok(books)
    }
}

// ---------------------------------------------------------------------------
// ABS REST response types (only the fields Libro needs; unknown fields ignored).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AbsLibrariesResponse {
    #[serde(default)]
    libraries: Vec<AbsLibrary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbsLibrary {
    id: String,
    #[allow(dead_code)]
    #[serde(default)]
    name: String,
    /// `"book"` or `"podcast"`.
    #[serde(default)]
    media_type: String,
}

#[derive(Debug, Deserialize)]
struct AbsLibraryItemsResponse {
    #[serde(default)]
    results: Vec<AbsLibraryItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbsLibraryItem {
    id: String,
    #[serde(default)]
    media: AbsMedia,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbsMedia {
    #[serde(default)]
    metadata: AbsMetadata,
    #[serde(default)]
    cover_path: Option<String>,
    /// Number of audio tracks (present on minified media). `> 0` ⇒ audiobook.
    #[serde(default)]
    num_tracks: Option<u32>,
    /// Ebook format (e.g. `"epub"`) when the item is/also-is an ebook.
    #[serde(default)]
    ebook_format: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbsMetadata {
    #[serde(default)]
    title: Option<String>,
    /// Minified metadata folds authors into a comma-separated string.
    #[serde(default)]
    author_name: Option<String>,
    #[serde(default)]
    series_name: Option<String>,
    #[serde(default)]
    isbn: Option<String>,
    #[serde(default)]
    asin: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AbsMe {
    #[serde(default, rename = "mediaProgress")]
    media_progress: Vec<AbsMediaProgress>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbsMediaProgress {
    #[serde(default)]
    library_item_id: String,
    /// Present for podcast episodes; absent for books.
    #[serde(default)]
    episode_id: Option<String>,
    #[serde(default)]
    progress: f32,
    #[serde(default)]
    current_time: f64,
    #[serde(default)]
    is_finished: bool,
}

// ---------------------------------------------------------------------------
// Pure mapping helpers (unit-tested without any network access).
// ---------------------------------------------------------------------------

/// Decide the normalized [`MediaType`] for an ABS item.
fn media_type_for(item: &AbsLibraryItem, library_media_type: &str) -> MediaType {
    if library_media_type.eq_ignore_ascii_case("podcast") {
        MediaType::Podcast
    } else if item.media.num_tracks.unwrap_or(0) > 0 {
        MediaType::Audiobook
    } else if item.media.ebook_format.is_some() {
        MediaType::Ebook
    } else {
        // A "book" library item with no tracks and no ebook format is most
        // likely an audiobook still being scanned; default accordingly.
        MediaType::Audiobook
    }
}

/// Map one ABS library item into a normalized [`Book`].
fn map_item_to_book(
    item: &AbsLibraryItem,
    library_media_type: &str,
    base_url: &str,
    token: &str,
    source_provider_id: &str,
) -> Book {
    let md = &item.media.metadata;

    let mut book = Book::new(
        item.id.clone(),
        md.title.clone().unwrap_or_else(|| "Untitled".to_string()),
        media_type_for(item, library_media_type),
        source_provider_id,
    );

    book.authors = md
        .author_name
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect()
        })
        .unwrap_or_default();

    book.series = md.series_name.clone().filter(|s| !s.is_empty());

    if item.media.cover_path.is_some() {
        // Cover is served by the item cover endpoint; token as query string so
        // the URL is directly usable by an <img> tag in the frontend.
        book.cover_url = Some(format!(
            "{base_url}/api/items/{}/cover?token={token}",
            item.id
        ));
    }

    if let Some(isbn) = md.isbn.as_deref().filter(|s| !s.is_empty()) {
        book.identifiers.insert("isbn".into(), isbn.to_string());
    }
    if let Some(asin) = md.asin.as_deref().filter(|s| !s.is_empty()) {
        book.identifiers.insert("asin".into(), asin.to_string());
    }

    book
}

/// Merge ABS media progress into the matching [`Book`]s (by library item id).
///
/// Podcast episode progress (entries carrying an `episodeId`) is ignored here;
/// per-episode progress will be modeled in the audiobook-playback phase.
fn merge_progress(books: &mut [Book], progress: &[AbsMediaProgress]) {
    for p in progress {
        if p.episode_id.is_some() {
            continue;
        }
        if let Some(book) = books.iter_mut().find(|b| b.id == p.library_item_id) {
            book.progress = Some(Progress {
                fraction: p.progress,
                position_seconds: Some(p.current_time),
                finished: p.is_finished,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Representative minified `/api/libraries/{id}/items` payload, shaped after
    // the public ABS API docs (https://api.audiobookshelf.org/).
    fn items_fixture() -> &'static str {
        r#"{
          "results": [
            {
              "id": "li_bufnnmp4y5o2gbbxfm",
              "mediaType": "book",
              "media": {
                "metadata": {
                  "title": "The Fellowship of the Ring",
                  "authorName": "J. R. R. Tolkien, Someone Else",
                  "seriesName": "The Lord of the Rings",
                  "isbn": "9780547928210",
                  "asin": "B007978NPG"
                },
                "coverPath": "/metadata/items/li_bufnnmp4y5o2gbbxfm/cover.jpg",
                "numTracks": 12
              }
            },
            {
              "id": "li_ebookonly001",
              "mediaType": "book",
              "media": {
                "metadata": { "title": "An Ebook", "authorName": "Ann Author" },
                "ebookFormat": "epub"
              }
            }
          ],
          "total": 2
        }"#
    }

    fn me_fixture() -> &'static str {
        r#"{
          "id": "root",
          "username": "root",
          "mediaProgress": [
            {
              "libraryItemId": "li_bufnnmp4y5o2gbbxfm",
              "duration": 1454.18,
              "progress": 0.4349,
              "currentTime": 632.56,
              "isFinished": false
            },
            {
              "libraryItemId": "li_podcast",
              "episodeId": "ep_abc",
              "progress": 0.9,
              "currentTime": 10.0,
              "isFinished": false
            }
          ]
        }"#
    }

    #[test]
    fn maps_audiobook_item_with_authors_series_identifiers_and_cover() {
        let resp: AbsLibraryItemsResponse = serde_json::from_str(items_fixture()).unwrap();
        let book = map_item_to_book(
            &resp.results[0],
            "book",
            "https://abs.example.com",
            "TESTTOKEN",
            AudiobookshelfProvider::ID,
        );

        assert_eq!(book.id, "li_bufnnmp4y5o2gbbxfm");
        assert_eq!(book.title, "The Fellowship of the Ring");
        assert_eq!(book.media_type, MediaType::Audiobook);
        assert_eq!(book.authors, vec!["J. R. R. Tolkien", "Someone Else"]);
        assert_eq!(book.series.as_deref(), Some("The Lord of the Rings"));
        assert_eq!(book.source_provider_id, "audiobookshelf");
        assert_eq!(
            book.identifiers.get("isbn").map(String::as_str),
            Some("9780547928210")
        );
        assert_eq!(
            book.identifiers.get("asin").map(String::as_str),
            Some("B007978NPG")
        );
        assert_eq!(
            book.cover_url.as_deref(),
            Some("https://abs.example.com/api/items/li_bufnnmp4y5o2gbbxfm/cover?token=TESTTOKEN")
        );
    }

    #[test]
    fn detects_ebook_and_has_no_cover_when_cover_path_absent() {
        let resp: AbsLibraryItemsResponse = serde_json::from_str(items_fixture()).unwrap();
        let book = map_item_to_book(
            &resp.results[1],
            "book",
            "https://abs.example.com",
            "TESTTOKEN",
            AudiobookshelfProvider::ID,
        );
        assert_eq!(book.media_type, MediaType::Ebook);
        assert!(book.cover_url.is_none());
        assert_eq!(book.authors, vec!["Ann Author"]);
        assert!(book.identifiers.is_empty());
    }

    #[test]
    fn podcast_library_maps_to_podcast_media_type() {
        let resp: AbsLibraryItemsResponse = serde_json::from_str(items_fixture()).unwrap();
        let book = map_item_to_book(
            &resp.results[0],
            "podcast",
            "https://abs.example.com",
            "T",
            AudiobookshelfProvider::ID,
        );
        assert_eq!(book.media_type, MediaType::Podcast);
    }

    #[test]
    fn merges_book_progress_and_skips_podcast_episode_progress() {
        let items: AbsLibraryItemsResponse = serde_json::from_str(items_fixture()).unwrap();
        let me: AbsMe = serde_json::from_str(me_fixture()).unwrap();

        let mut books: Vec<Book> = items
            .results
            .iter()
            .map(|i| map_item_to_book(i, "book", "https://abs.example.com", "T", "audiobookshelf"))
            .collect();

        merge_progress(&mut books, &me.media_progress);

        let progressed = &books[0];
        let p = progressed.progress.as_ref().expect("progress merged");
        assert!((p.fraction - 0.4349).abs() < 1e-4);
        assert_eq!(p.position_seconds, Some(632.56));
        assert!(!p.finished);

        // The ebook item had no matching progress entry.
        assert!(books[1].progress.is_none());
    }
}
