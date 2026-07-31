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
//! * `POST /api/items/{id}/play`       — open a playback session (audio tracks +
//!                                       chapters) for the audiobook player.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::{Book, MediaType, Progress};
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

/// A single audiobook chapter marker (normalized, provider-agnostic).
///
/// Times are in seconds from the start of the (logical) audiobook. Consumed by
/// the frontend audio player for the chapter list / jump-to-chapter control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioChapter {
    pub id: u32,
    /// Start offset in seconds.
    pub start: f64,
    /// End offset in seconds.
    pub end: f64,
    pub title: String,
}

/// A normalized, directly-playable audiobook stream + its chapter list.
///
/// This is what [`crate::providers::audiobookshelf::AudiobookshelfProvider::resolve_playback`]
/// produces and the Tauri `get_audiobook_stream` command returns to the player.
/// The player only needs a URL an `<audio>` element can load, so the auth token
/// is embedded in `stream_url`'s query string (an HTML media element cannot send
/// an `Authorization` header — see the note in [`map_playback_session`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlayback {
    /// Absolute, directly-playable stream URL (auth token in the query string).
    pub stream_url: String,
    /// Total duration in seconds, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// MIME type hint for the stream (e.g. `"audio/mpeg"`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Chapter markers for the jump-to-chapter UI (may be empty).
    #[serde(default)]
    pub chapters: Vec<AudioChapter>,
}

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

    /// Open a playback session for one library item and resolve it into a
    /// normalized, directly-playable [`AudioPlayback`] (stream URL + chapters).
    ///
    /// Uses `POST /api/items/{id}/play`, which returns the item's audio tracks
    /// and chapter markers. The heavy lifting (URL resolution, chapter mapping)
    /// is delegated to the pure, unit-tested [`map_playback_session`] helper.
    ///
    /// TODO(live): this path needs verification against a running ABS server —
    /// there is none in the build/CI environment, so only the pure mapping is
    /// exercised by tests. See `ARCHITECTURE.md` → audiobook playback.
    pub async fn resolve_playback(&self, item_id: &str) -> ProviderResult<AudioPlayback> {
        if self.base().is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }
        let url = format!("{}/api/items/{item_id}/play", self.base());
        let resp = self
            .client
            .post(&url)
            // ABS accepts an empty JSON body; defaults pick the item's tracks.
            .json(&serde_json::json!({}))
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => {
                let session: AbsPlaybackSession = resp.json().await.map_err(|e| {
                    ProviderError::Other(format!("invalid play-session response: {e}"))
                })?;
                map_playback_session(&session, self.base(), &self.config.api_token)
            }
            401 | 403 => Err(ProviderError::NotAuthenticated),
            404 => Err(ProviderError::Api(format!("item {item_id} not found"))),
            other => Err(ProviderError::Other(format!(
                "unexpected status {other} from {url}"
            ))),
        }
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
                locator: None,
                finished: p.is_finished,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// ABS playback-session response types (`POST /api/items/{id}/play`).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbsPlaybackSession {
    #[serde(default)]
    audio_tracks: Vec<AbsAudioTrack>,
    #[serde(default)]
    chapters: Vec<AbsChapter>,
    /// Total duration of the item in seconds, if the session reports it.
    #[serde(default)]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbsAudioTrack {
    /// Server-relative URL for the track's audio content,
    /// e.g. `/api/items/{id}/file/{ino}` (may already carry query params).
    #[serde(default)]
    content_url: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AbsChapter {
    #[serde(default)]
    id: u32,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
    #[serde(default)]
    title: Option<String>,
}

/// Resolve an ABS play session into a normalized [`AudioPlayback`].
///
/// Pure and unit-tested (no network). Responsibilities:
///   * pick a playable audio track and resolve its (possibly server-relative)
///     `contentUrl` to an **absolute** URL against `base_url`;
///   * embed the auth `token` in the URL's query string, because an HTML
///     `<audio>` element cannot send an `Authorization: Bearer` header — ABS
///     accepts `?token=` for this reason;
///   * carry the chapter markers and duration through.
///
/// v1 uses the **first** audio track. Multi-file audiobooks (ABS returns one
/// track per source file) therefore expose only their first file for now;
/// gapless multi-track playback (a playlist/`MediaSource` queue, or the server's
/// merged/HLS stream) is a TODO — see `ARCHITECTURE.md`.
fn map_playback_session(
    session: &AbsPlaybackSession,
    base_url: &str,
    token: &str,
) -> ProviderResult<AudioPlayback> {
    let track = session
        .audio_tracks
        .first()
        .ok_or_else(|| ProviderError::Api("play session has no audio tracks".into()))?;

    let stream_url = resolve_stream_url(base_url, &track.content_url, token);

    let chapters = session
        .chapters
        .iter()
        .map(|c| AudioChapter {
            id: c.id,
            start: c.start,
            end: c.end,
            title: c
                .title
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("Chapter {}", c.id + 1)),
        })
        .collect();

    let duration = session
        .duration
        .or(track.duration)
        .filter(|d| *d > 0.0);

    Ok(AudioPlayback {
        stream_url,
        duration,
        mime_type: track.mime_type.clone().filter(|m| !m.is_empty()),
        chapters,
    })
}

/// Resolve a (possibly server-relative) content URL to an absolute stream URL
/// with the auth token appended as a query parameter.
fn resolve_stream_url(base_url: &str, content_url: &str, token: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let absolute = if content_url.starts_with("http://") || content_url.starts_with("https://") {
        content_url.to_string()
    } else if let Some(stripped) = content_url.strip_prefix('/') {
        format!("{base}/{stripped}")
    } else {
        format!("{base}/{content_url}")
    };
    if token.is_empty() {
        absolute
    } else {
        let sep = if absolute.contains('?') { '&' } else { '?' };
        format!("{absolute}{sep}token={token}")
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

    // Representative `POST /api/items/{id}/play` session payload, shaped after
    // the public ABS API docs.
    fn play_session_fixture() -> &'static str {
        r#"{
          "id": "play_session_abc",
          "duration": 3600.0,
          "audioTracks": [
            {
              "index": 1,
              "startOffset": 0,
              "duration": 3600.0,
              "title": "track1.mp3",
              "contentUrl": "/api/items/li_abc/file/aud1",
              "mimeType": "audio/mpeg"
            }
          ],
          "chapters": [
            { "id": 0, "start": 0.0, "end": 1200.0, "title": "Chapter One" },
            { "id": 1, "start": 1200.0, "end": 2400.0, "title": "" },
            { "id": 2, "start": 2400.0, "end": 3600.0, "title": "The End" }
          ]
        }"#
    }

    #[test]
    fn maps_play_session_to_absolute_stream_url_with_token_and_chapters() {
        let session: AbsPlaybackSession = serde_json::from_str(play_session_fixture()).unwrap();
        let pb = map_playback_session(&session, "https://abs.example.com/", "TESTTOKEN").unwrap();

        assert_eq!(
            pb.stream_url,
            "https://abs.example.com/api/items/li_abc/file/aud1?token=TESTTOKEN"
        );
        assert_eq!(pb.mime_type.as_deref(), Some("audio/mpeg"));
        assert_eq!(pb.duration, Some(3600.0));
        assert_eq!(pb.chapters.len(), 3);
        assert_eq!(pb.chapters[0].title, "Chapter One");
        assert!((pb.chapters[0].end - 1200.0).abs() < f64::EPSILON);
        // An empty chapter title falls back to a generated "Chapter N" label.
        assert_eq!(pb.chapters[1].title, "Chapter 2");
    }

    #[test]
    fn resolve_stream_url_appends_token_respecting_existing_query() {
        assert_eq!(
            resolve_stream_url("https://s/", "/api/x", "T"),
            "https://s/api/x?token=T"
        );
        // Relative without a leading slash.
        assert_eq!(
            resolve_stream_url("https://s", "api/x", "T"),
            "https://s/api/x?token=T"
        );
        // Existing query string uses `&`.
        assert_eq!(
            resolve_stream_url("https://s", "/api/x?a=1", "T"),
            "https://s/api/x?a=1&token=T"
        );
        // An already-absolute content URL is preserved.
        assert_eq!(
            resolve_stream_url("https://s", "https://cdn/y.mp3", "T"),
            "https://cdn/y.mp3?token=T"
        );
        // No token → no query param added.
        assert_eq!(resolve_stream_url("https://s", "/api/x", ""), "https://s/api/x");
    }

    #[test]
    fn play_session_without_audio_tracks_is_an_error_not_a_panic() {
        let session: AbsPlaybackSession =
            serde_json::from_str(r#"{ "audioTracks": [], "chapters": [] }"#).unwrap();
        let err = map_playback_session(&session, "https://s", "T").unwrap_err();
        assert!(matches!(err, ProviderError::Api(_)));
    }
}
