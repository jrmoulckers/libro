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
//! * `PATCH /api/me/progress/{id}`      — push the user's listening position
//!                                       back to the server (opt-in sync).

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

/// A normalized, directly-playable audiobook **manifest**: the full ordered list
/// of audio tracks laid out on one **book-absolute timeline**, plus chapters and
/// total duration.
///
/// This is what [`crate::providers::audiobookshelf::AudiobookshelfProvider::resolve_playback`]
/// produces and the Tauri `get_audiobook_stream` command returns to the player.
///
/// An ABS audiobook is typically **one track per source file**. Rather than
/// exposing only the first file, the manifest carries every track with a
/// cumulative `start_offset_seconds`, so the player can present a single
/// continuous timeline (seek/skip/chapter-jump across track boundaries) and
/// auto-advance from one track to the next. The auth token is embedded in each
/// track's `url` query string (an HTML `<audio>` element cannot send an
/// `Authorization` header — see [`map_playback_session`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackManifest {
    /// Ordered audio tracks with book-absolute start offsets.
    pub tracks: Vec<PlaybackTrack>,
    /// Chapter markers for the jump-to-chapter UI (book-absolute times; may be
    /// empty). A chapter may span a track boundary.
    #[serde(default)]
    pub chapters: Vec<AudioChapter>,
    /// Total book duration in seconds = the sum of the track durations.
    pub total_duration: f64,
}

/// A single audio track (one source file) positioned on the book-absolute
/// timeline. The player maps a whole-book position to `(track, offset-within)`
/// via `start_offset_seconds`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackTrack {
    /// 0-based position of this track in the book.
    pub index: usize,
    /// Absolute, directly-playable stream URL (auth token in the query string).
    pub url: String,
    /// This track's own duration in seconds.
    pub duration_seconds: f64,
    /// Cumulative start offset in seconds = sum of all prior track durations.
    /// The book-absolute position while this track plays is
    /// `start_offset_seconds + audio.currentTime`.
    pub start_offset_seconds: f64,
    /// MIME type hint for the stream (e.g. `"audio/mpeg"`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
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
    /// Opt-in: mirror the in-app player's local listening position **up** to this
    /// ABS server (`PATCH /api/me/progress/{id}`). Default `false` — writing to
    /// the user's server must never be a silent side effect of pressing play.
    /// See [`crate::listening_sync`].
    #[serde(default)]
    pub sync_listening_progress: bool,
    /// Opt-in: pull the server's listening progress **down** on library load and
    /// reconcile it against the local store, so a book listened on another device
    /// resumes here. Default `false`. See [`crate::progress_sync`].
    #[serde(default)]
    pub pull_progress: bool,
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
    /// normalized, directly-playable [`PlaybackManifest`] (all tracks on a
    /// book-absolute timeline + chapters).
    ///
    /// Uses `POST /api/items/{id}/play`, which returns the item's audio tracks
    /// and chapter markers. The heavy lifting (URL resolution, timeline assembly,
    /// chapter mapping) is delegated to the pure, unit-tested
    /// [`map_playback_session`] helper.
    ///
    /// TODO(live): this path needs verification against a running ABS server —
    /// there is none in the build/CI environment, so only the pure mapping is
    /// exercised by tests. See `ARCHITECTURE.md` → audiobook playback.
    pub async fn resolve_playback(&self, item_id: &str) -> ProviderResult<PlaybackManifest> {
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

    /// Push the user's listening position for one library item back to the
    /// server: `PATCH /api/me/progress/{libraryItemId}` (Bearer auth) with a body
    /// carrying `currentTime`, `duration`, `progress` (fraction), and
    /// `isFinished`. The request body is built by the pure, unit-tested
    /// [`map_media_progress_body`] helper (kept separate from the HTTP call, like
    /// [`map_playback_session`]).
    ///
    /// Best-effort by contract: used through the [`crate::listening_sync`] engine,
    /// which swallows any error so it can never disturb the local (source-of-truth)
    /// listening store or the player.
    ///
    /// TODO(live): there is no ABS server in this build environment, so only the
    /// pure body mapping is exercised by tests — live verification against a
    /// running instance is pending. See `ARCHITECTURE.md` → audiobook playback.
    pub async fn update_media_progress(
        &self,
        item_id: &str,
        position_seconds: f64,
        duration_seconds: Option<f64>,
        is_finished: bool,
    ) -> ProviderResult<()> {
        if self.base().is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }
        let url = format!("{}/api/me/progress/{item_id}", self.base());
        let body = map_media_progress_body(position_seconds, duration_seconds, is_finished);
        let resp = self
            .client
            .patch(&url)
            .json(&body)
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 | 204 => Ok(()),
            401 | 403 => Err(ProviderError::NotAuthenticated),
            404 => Err(ProviderError::Api(format!("item {item_id} not found"))),
            other => Err(ProviderError::Other(format!(
                "unexpected status {other} from {url}"
            ))),
        }
    }

    /// Read the user's current listening progress for one library item from the
    /// server, for the inbound reconciliation pass ([`crate::progress_sync`]).
    ///
    /// Fetches `GET /api/me` (whose payload carries the user's `mediaProgress`
    /// list) and returns the entry matching `item_id`, mapped by the pure,
    /// unit-tested [`map_abs_progress_record`] helper. `Ok(None)` when the server
    /// has no progress for the item.
    ///
    /// TODO(live): needs a running ABS server to verify end to end; only the pure
    /// mapping is exercised by tests here.
    pub async fn fetch_media_progress(
        &self,
        item_id: &str,
    ) -> ProviderResult<Option<crate::progress_sync::RemoteProgress>> {
        if self.base().is_empty() {
            return Err(ProviderError::Config("base_url is empty".into()));
        }
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
                let me: AbsMe = resp
                    .json()
                    .await
                    .map_err(|e| ProviderError::Other(format!("invalid /api/me response: {e}")))?;
                Ok(me
                    .media_progress
                    .iter()
                    .find(|p| p.episode_id.is_none() && p.library_item_id == item_id)
                    .map(map_abs_progress_record))
            }
            401 | 403 => Err(ProviderError::NotAuthenticated),
            other => Err(ProviderError::Other(format!(
                "unexpected status {other} from {url}"
            ))),
        }
    }
}

/// ABS as a progress *source* for inbound reconciliation (see
/// [`crate::progress_sync`]). Only audiobook items sourced from ABS are pulled;
/// the audiobook `Book.id` is the ABS `libraryItemId`.
#[async_trait]
impl crate::progress_sync::ProgressSource for AudiobookshelfProvider {
    async fn fetch_remote_progress(
        &self,
        book: &crate::models::Book,
    ) -> ProviderResult<Option<crate::progress_sync::RemoteProgress>> {
        self.fetch_media_progress(&book.id).await
    }
}

/// Map an ABS `mediaProgress` record into a normalized
/// [`RemoteProgress`](crate::progress_sync::RemoteProgress). Pure and
/// unit-tested; ABS reports `lastUpdate` in **milliseconds**, normalized here to
/// epoch **seconds** for the reconciliation policy.
fn map_abs_progress_record(p: &AbsMediaProgress) -> crate::progress_sync::RemoteProgress {
    crate::progress_sync::RemoteProgress {
        fraction: p.progress,
        position_seconds: Some(p.current_time),
        finished: p.is_finished,
        updated_at: p.last_update.map(|ms| ms / 1000),
    }
}

/// ABS as a listening-progress sink for the in-app player (see
/// [`crate::listening_sync`]). Adapts the connector's media-progress write to the
/// [`ListeningTracker`](crate::listening_sync::ListeningTracker) contract so the
/// sync engine stays connector-agnostic and testable against a fake.
#[async_trait]
impl crate::listening_sync::ListeningTracker for AudiobookshelfProvider {
    async fn update_media_progress(
        &self,
        item_id: &str,
        position_seconds: f64,
        duration_seconds: Option<f64>,
        is_finished: bool,
    ) -> ProviderResult<()> {
        // Disambiguate from the inherent method of the same name.
        AudiobookshelfProvider::update_media_progress(
            self,
            item_id,
            position_seconds,
            duration_seconds,
            is_finished,
        )
        .await
    }
}

/// Build the `PATCH /api/me/progress/{id}` request body from a listening
/// position. Pure and unit-tested (no network), mirroring [`map_playback_session`].
///
/// Emits `currentTime` (seconds) and `isFinished` always; adds `duration` and the
/// derived `progress` fraction (`currentTime / duration`, clamped to `0.0..=1.0`)
/// when a positive duration is known. A finished item reports `progress: 1.0`.
pub fn map_media_progress_body(
    position_seconds: f64,
    duration_seconds: Option<f64>,
    is_finished: bool,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    body.insert(
        "currentTime".into(),
        serde_json::json!(position_seconds.max(0.0)),
    );
    body.insert("isFinished".into(), serde_json::json!(is_finished));

    if let Some(dur) = duration_seconds.filter(|d| *d > 0.0) {
        body.insert("duration".into(), serde_json::json!(dur));
        let fraction = if is_finished {
            1.0
        } else {
            (position_seconds / dur).clamp(0.0, 1.0)
        };
        body.insert("progress".into(), serde_json::json!(fraction));
    } else if is_finished {
        body.insert("progress".into(), serde_json::json!(1.0));
    }

    serde_json::Value::Object(body)
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
    /// Server last-update time in **milliseconds** since the epoch. Drives the
    /// newest-wins branch of the inbound reconciliation ([`crate::progress_sync`]).
    #[serde(default)]
    last_update: Option<i64>,
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

/// Resolve an ABS play session into a normalized [`PlaybackManifest`].
///
/// Pure and unit-tested (no network). Responsibilities:
///   * resolve **every** track's (possibly server-relative) `contentUrl` to an
///     **absolute** URL against `base_url`, embedding the auth `token` in the
///     query string (an HTML `<audio>` element cannot send an
///     `Authorization: Bearer` header — ABS accepts `?token=` for this reason);
///   * lay the tracks out on a **book-absolute timeline** via the pure
///     [`assemble_timeline`] helper (cumulative `start_offset_seconds` +
///     `total_duration`);
///   * carry the chapter markers through (already book-absolute in ABS).
///
/// Returns **all** tracks (ABS emits one per source file), so the player can
/// present one continuous book and auto-advance across track boundaries. An
/// empty track list is an error, not a panic.
fn map_playback_session(
    session: &AbsPlaybackSession,
    base_url: &str,
    token: &str,
) -> ProviderResult<PlaybackManifest> {
    if session.audio_tracks.is_empty() {
        return Err(ProviderError::Api("play session has no audio tracks".into()));
    }

    let resolved: Vec<(String, f64, Option<String>)> = session
        .audio_tracks
        .iter()
        .map(|t| {
            (
                resolve_stream_url(base_url, &t.content_url, token),
                t.duration.unwrap_or(0.0),
                t.mime_type.clone().filter(|m| !m.is_empty()),
            )
        })
        .collect();

    let (tracks, total_duration) = assemble_timeline(resolved);

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

    Ok(PlaybackManifest {
        tracks,
        chapters,
        total_duration,
    })
}

/// Lay an ordered list of `(url, duration_seconds, mime_type)` tracks onto a
/// book-absolute timeline.
///
/// Pure and unit-tested. Each track's `start_offset_seconds` is the cumulative
/// sum of all **prior** track durations, and the returned total is the sum of
/// every track's duration. A negative/`NaN`-ish duration is clamped to 0 so a
/// bad value can't rewind the timeline. This is the single source of truth for
/// the whole-book position the player exposes.
fn assemble_timeline(tracks: Vec<(String, f64, Option<String>)>) -> (Vec<PlaybackTrack>, f64) {
    let mut offset = 0.0_f64;
    let mut out = Vec::with_capacity(tracks.len());
    for (index, (url, duration, mime_type)) in tracks.into_iter().enumerate() {
        let dur = if duration.is_finite() && duration > 0.0 {
            duration
        } else {
            0.0
        };
        out.push(PlaybackTrack {
            index,
            url,
            duration_seconds: dur,
            start_offset_seconds: offset,
            mime_type,
        });
        offset += dur;
    }
    (out, offset)
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
    // the public ABS API docs. Multi-file: three tracks (one per source file),
    // with chapters that cross track boundaries.
    fn play_session_fixture() -> &'static str {
        r#"{
          "id": "play_session_abc",
          "audioTracks": [
            {
              "index": 0,
              "duration": 1200.0,
              "title": "part1.mp3",
              "contentUrl": "/api/items/li_abc/file/aud1",
              "mimeType": "audio/mpeg"
            },
            {
              "index": 1,
              "duration": 1500.0,
              "title": "part2.mp3",
              "contentUrl": "/api/items/li_abc/file/aud2",
              "mimeType": "audio/mpeg"
            },
            {
              "index": 2,
              "duration": 900.0,
              "title": "part3.mp3",
              "contentUrl": "/api/items/li_abc/file/aud3",
              "mimeType": "audio/mpeg"
            }
          ],
          "chapters": [
            { "id": 0, "start": 0.0, "end": 1000.0, "title": "Chapter One" },
            { "id": 1, "start": 1000.0, "end": 2000.0, "title": "" },
            { "id": 2, "start": 2000.0, "end": 3600.0, "title": "The End" }
          ]
        }"#
    }

    #[test]
    fn maps_play_session_to_multi_track_manifest_with_unified_timeline() {
        let session: AbsPlaybackSession = serde_json::from_str(play_session_fixture()).unwrap();
        let pb = map_playback_session(&session, "https://abs.example.com/", "TESTTOKEN").unwrap();

        // All three tracks are returned (not just the first).
        assert_eq!(pb.tracks.len(), 3);

        // Each track's URL is absolute with the auth token in the query string.
        assert_eq!(
            pb.tracks[0].url,
            "https://abs.example.com/api/items/li_abc/file/aud1?token=TESTTOKEN"
        );
        assert_eq!(
            pb.tracks[2].url,
            "https://abs.example.com/api/items/li_abc/file/aud3?token=TESTTOKEN"
        );

        // Cumulative book-absolute offsets: 0, 1200, 2700.
        assert_eq!(pb.tracks[0].index, 0);
        assert!((pb.tracks[0].start_offset_seconds - 0.0).abs() < f64::EPSILON);
        assert!((pb.tracks[1].start_offset_seconds - 1200.0).abs() < f64::EPSILON);
        assert!((pb.tracks[2].start_offset_seconds - 2700.0).abs() < f64::EPSILON);

        // total_duration = sum of track durations.
        assert!((pb.total_duration - 3600.0).abs() < f64::EPSILON);

        // Chapters are preserved book-absolute (chapter 1 spans the 1200s boundary).
        assert_eq!(pb.chapters.len(), 3);
        assert_eq!(pb.chapters[0].title, "Chapter One");
        assert_eq!(pb.chapters[1].title, "Chapter 2"); // empty title → generated
        assert!((pb.chapters[1].start - 1000.0).abs() < f64::EPSILON);
        assert!((pb.chapters[1].end - 2000.0).abs() < f64::EPSILON);
        assert_eq!(pb.tracks[0].mime_type.as_deref(), Some("audio/mpeg"));
    }

    #[test]
    fn assemble_timeline_computes_cumulative_offsets_and_total() {
        // Empty → no tracks, zero total.
        let (empty, total) = assemble_timeline(vec![]);
        assert!(empty.is_empty());
        assert_eq!(total, 0.0);

        // Single track.
        let (one, total) = assemble_timeline(vec![("u0".into(), 42.0, None)]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].start_offset_seconds, 0.0);
        assert_eq!(total, 42.0);

        // Many tracks accumulate.
        let (many, total) = assemble_timeline(vec![
            ("u0".into(), 10.0, Some("audio/mpeg".into())),
            ("u1".into(), 20.0, None),
            ("u2".into(), 5.0, None),
        ]);
        assert_eq!(many[0].start_offset_seconds, 0.0);
        assert_eq!(many[1].start_offset_seconds, 10.0);
        assert_eq!(many[2].start_offset_seconds, 30.0);
        assert_eq!(many[1].index, 1);
        assert_eq!(total, 35.0);
    }

    #[test]
    fn assemble_timeline_clamps_bad_durations_so_offsets_never_rewind() {
        // A zero/negative/non-finite duration contributes 0 and does not move the
        // next track's offset backwards.
        let (tracks, total) = assemble_timeline(vec![
            ("u0".into(), 10.0, None),
            ("u1".into(), 0.0, None),
            ("u2".into(), -5.0, None),
            ("u3".into(), f64::NAN, None),
            ("u4".into(), 7.0, None),
        ]);
        assert_eq!(tracks[0].start_offset_seconds, 0.0);
        assert_eq!(tracks[1].start_offset_seconds, 10.0);
        assert_eq!(tracks[2].start_offset_seconds, 10.0);
        assert_eq!(tracks[3].start_offset_seconds, 10.0);
        assert_eq!(tracks[4].start_offset_seconds, 10.0);
        assert_eq!(tracks[2].duration_seconds, 0.0);
        assert_eq!(tracks[3].duration_seconds, 0.0);
        assert_eq!(total, 17.0);
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

    #[test]
    fn media_progress_body_carries_current_time_duration_and_fraction() {
        let body = map_media_progress_body(1800.0, Some(3600.0), false);
        assert_eq!(body["currentTime"], serde_json::json!(1800.0));
        assert_eq!(body["duration"], serde_json::json!(3600.0));
        assert_eq!(body["isFinished"], serde_json::json!(false));
        assert_eq!(body["progress"], serde_json::json!(0.5));
    }

    #[test]
    fn media_progress_body_marks_finished_as_full_progress() {
        let body = map_media_progress_body(3600.0, Some(3600.0), true);
        assert_eq!(body["isFinished"], serde_json::json!(true));
        assert_eq!(body["progress"], serde_json::json!(1.0));
    }

    #[test]
    fn media_progress_body_without_duration_omits_duration_and_fraction() {
        let body = map_media_progress_body(120.0, None, false);
        assert_eq!(body["currentTime"], serde_json::json!(120.0));
        assert_eq!(body["isFinished"], serde_json::json!(false));
        assert!(body.get("duration").is_none());
        assert!(body.get("progress").is_none());
    }

    #[test]
    fn abs_progress_record_normalizes_millis_to_seconds() {
        let rec = AbsMediaProgress {
            library_item_id: "li_1".into(),
            episode_id: None,
            progress: 0.42,
            current_time: 1512.0,
            is_finished: false,
            last_update: Some(1_700_000_000_000), // ms
        };
        let mapped = map_abs_progress_record(&rec);
        assert!((mapped.fraction - 0.42).abs() < 1e-6);
        assert_eq!(mapped.position_seconds, Some(1512.0));
        assert!(!mapped.finished);
        assert_eq!(mapped.updated_at, Some(1_700_000_000)); // seconds
    }

    #[test]
    fn abs_progress_record_without_last_update_has_no_timestamp() {
        let rec = AbsMediaProgress {
            library_item_id: "li_1".into(),
            episode_id: None,
            progress: 1.0,
            current_time: 3600.0,
            is_finished: true,
            last_update: None,
        };
        let mapped = map_abs_progress_record(&rec);
        assert!(mapped.finished);
        assert_eq!(mapped.updated_at, None);
    }
}
