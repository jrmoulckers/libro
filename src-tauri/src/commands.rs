//! Tauri command surface exposed to the frontend.

use libro_core::config::{self, AppConfig, ListeningStore, ProviderConfig, ReadingStore};
use libro_core::kindle::{
    is_configured as kindle_is_configured, send_epub_to_kindle, KindleConfig, SendOutcome,
    SmtpKindleSender,
};
use libro_core::metadata::{enrich_catalog, BookMetadata, EnrichOptions, MetadataRegistry};
use libro_core::models::{Book, Progress};
use libro_core::providers::audiobookshelf::{
    AudiobookshelfConfig, AudiobookshelfProvider, PlaybackManifest,
};
use libro_core::providers::hardcover::{HardcoverConfig, HardcoverProvider};
use libro_core::providers::lazylibrarian::{LazyLibrarianConfig, LazyLibrarianProvider};
use libro_core::providers::libby::{LibbyConfig, LibbyProvider};
use libro_core::providers::localfiles::{
    extract_cover, read_book_file, LocalFilesConfig, LocalFilesProvider,
};
use libro_core::providers::opds::{OpdsConfig, OpdsProvider};
use libro_core::providers::Provider;
use libro_core::plugins::{load_plugins, PluginKind, PluginProvider, WasmPluginProvider};
use libro_core::sync::{sync_reading_progress, ReadingSyncState, SyncOutcome};
use libro_core::listening_sync::{
    sync_listening_progress, ListeningSyncOutcome, ListeningSyncState,
};
use libro_core::progress_sync::{
    lane_for, reconcile_catalog_with_policy, ConflictChoice, ConflictLane, Lane, ProgressConflict,
    ProgressSource, ProgressStoreLike, ReconcileReport, SyncLane,
};

use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory store of progress conflicts awaiting manual resolution, keyed by
/// `book_id`. Populated by [`reconcile_progress`] when the policy is
/// [`ConflictResolution::Manual`]; read by [`list_progress_conflicts`] and
/// drained by [`resolve_progress_conflict`]. Best-effort UI state — it is not
/// persisted and is rebuilt on each reconcile pass.
#[derive(Default)]
pub struct ConflictState(Mutex<HashMap<String, ProgressConflict>>);

impl ConflictState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the entire pending set with the latest reconcile pass's result.
    fn replace_all(&self, items: Vec<ProgressConflict>) {
        let mut guard = self.0.lock().unwrap();
        guard.clear();
        for c in items {
            guard.insert(c.book_id.clone(), c);
        }
    }

    /// Snapshot the pending conflicts for the UI.
    fn list(&self) -> Vec<ProgressConflict> {
        self.0.lock().unwrap().values().cloned().collect()
    }

    /// Remove and return one pending conflict (on resolve).
    fn take(&self, book_id: &str) -> Option<ProgressConflict> {
        self.0.lock().unwrap().remove(book_id)
    }
}

/// Max concurrent remote progress fetches in [`reconcile_progress`]. Bounded so a
/// large library never bursts the ABS/Hardcover APIs (mirrors the metadata
/// enrichment pass's `buffer_unordered` cap).
const RECONCILE_CONCURRENCY: usize = 6;

/// Instantiate the set of [`Provider`]s described by an [`AppConfig`].
///
/// This is the connector registry: adding a new connector means adding one arm
/// here that maps a `provider_type` string to its constructor. Unknown or
/// disabled providers are skipped. With no configured providers this returns an
/// empty list (the frontend then shows the empty/onboarding state).
fn build_providers(config: &AppConfig) -> Vec<Box<dyn Provider>> {
    let mut providers: Vec<Box<dyn Provider>> = Vec::new();

    // Discover installed plugins once. A plugin extends the connector registry
    // without recompiling core: a configured `provider_type` that matches a
    // loaded plugin id is instantiated as a declarative [`PluginProvider`],
    // capability-scoped and sandboxed to its manifest's allowed domains.
    let plugins = load_plugins(&config::data_dir().join("plugins"));

    for pc in config.providers.iter().filter(|p| p.enabled) {
        let settings = pc.settings.clone();
        match pc.provider_type.as_str() {
            AudiobookshelfProvider::ID => {
                let cfg: AudiobookshelfConfig =
                    serde_json::from_value(settings).unwrap_or_default();
                providers.push(Box::new(AudiobookshelfProvider::new(cfg)));
            }
            HardcoverProvider::ID => {
                let cfg: HardcoverConfig = serde_json::from_value(settings).unwrap_or_default();
                providers.push(Box::new(HardcoverProvider::new(cfg)));
            }
            LazyLibrarianProvider::ID => {
                let cfg: LazyLibrarianConfig =
                    serde_json::from_value(settings).unwrap_or_default();
                providers.push(Box::new(LazyLibrarianProvider::new(cfg)));
            }
            LibbyProvider::ID => {
                let cfg: LibbyConfig = serde_json::from_value(settings).unwrap_or_default();
                providers.push(Box::new(LibbyProvider::new(cfg)));
            }
            LocalFilesProvider::ID => {
                let cfg: LocalFilesConfig = serde_json::from_value(settings).unwrap_or_default();
                providers.push(Box::new(LocalFilesProvider::new(cfg)));
            }
            OpdsProvider::ID => {
                let cfg: OpdsConfig = serde_json::from_value(settings).unwrap_or_default();
                providers.push(Box::new(OpdsProvider::new(cfg)));
            }
            other => {
                // Not a native connector — is it an installed plugin?
                if let Some(loaded) = plugins.get(other) {
                    match loaded.manifest.kind() {
                        Ok(PluginKind::Declarative) => {
                            providers.push(Box::new(PluginProvider::new(
                                loaded.manifest.clone(),
                                settings,
                            )));
                        }
                        Ok(PluginKind::Wasm) => match &loaded.wasm_bytes {
                            Some(bytes) => providers.push(Box::new(WasmPluginProvider::new(
                                loaded.manifest.clone(),
                                bytes.clone(),
                                settings,
                            ))),
                            None => eprintln!(
                                "libro: wasm plugin '{other}' has no loaded module bytes, skipping"
                            ),
                        },
                        Err(e) => eprintln!("libro: plugin '{other}' has an invalid kind: {e}"),
                    }
                } else {
                    // Unknown connector type; ignore for now. A later phase may
                    // surface this to the user as a config warning.
                    eprintln!("libro: unknown provider_type '{other}', skipping");
                }
            }
        }
    }

    providers
}

/// A per-provider outcome returned to the frontend.
///
/// This lets the UI show a per-provider error state (task 2) instead of failing
/// the whole aggregation when one connector is misconfigured or offline.
#[derive(Debug, serde::Serialize)]
pub struct ProviderBooks {
    pub provider_id: String,
    pub display_name: String,
    /// Raw capability bits (see [`libro_core::providers::ProviderCapabilities`]).
    pub capabilities: u32,
    pub books: Vec<Book>,
    /// `None` on success; a human-readable message on failure.
    pub error: Option<String>,
}

/// Aggregate every configured provider's library into one normalized catalog.
///
/// This is the core aggregation pattern: fan out over each [`Provider`],
/// authenticate, pull its library, and merge the results. Per-provider failures
/// are logged and skipped so one broken connector can't sink the whole catalog.
///
/// The merged list is then run through the metadata-enrichment pass (Open
/// Library / Google Books) to backfill missing covers, descriptions, series, and
/// identifiers — unless disabled via `metadata.enrich_catalog`.
#[tauri::command]
pub async fn list_all_books() -> Result<Vec<Book>, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    let per_provider = collect_all(&app_config).await?;
    let books: Vec<Book> = per_provider.into_iter().flat_map(|p| p.books).collect();

    if app_config.metadata.enrich_catalog {
        let registry = MetadataRegistry::from_config(&app_config);
        Ok(enrich_catalog(&registry, books, &EnrichOptions::default()).await)
    } else {
        Ok(books)
    }
}

/// Like [`list_all_books`] but preserves per-provider results and errors so the
/// frontend can render a per-provider status/error state.
#[tauri::command]
pub async fn list_books_by_provider() -> Result<Vec<ProviderBooks>, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    collect_all(&app_config).await
}

/// Shared aggregation: build providers, authenticate + list each.
async fn collect_all(app_config: &AppConfig) -> Result<Vec<ProviderBooks>, String> {
    let mut providers = build_providers(app_config);

    // Look up each provider's stored settings blob by type for authentication.
    let settings_for = |provider_id: &str| -> serde_json::Value {
        app_config
            .providers
            .iter()
            .find(|p: &&ProviderConfig| p.provider_type == provider_id)
            .map(|p| p.settings.clone())
            .unwrap_or(serde_json::Value::Null)
    };

    let mut out: Vec<ProviderBooks> = Vec::new();
    for provider in providers.iter_mut() {
        let provider_id = provider.id().to_string();
        let display_name = provider.display_name().to_string();
        let capabilities = provider.capabilities().bits();

        let mut entry = ProviderBooks {
            provider_id: provider_id.clone(),
            display_name,
            capabilities,
            books: Vec::new(),
            error: None,
        };

        if let Err(e) = provider.authenticate(&settings_for(&provider_id)).await {
            eprintln!("libro: provider '{provider_id}' failed to authenticate: {e}");
            entry.error = Some(format!("authentication failed: {e}"));
            out.push(entry);
            continue;
        }
        match provider.list_library().await {
            Ok(books) => entry.books = books,
            Err(e) => {
                eprintln!("libro: provider '{provider_id}' list_library failed: {e}");
                entry.error = Some(e.to_string());
            }
        }
        out.push(entry);
    }

    Ok(out)
}

/// Free-text metadata search across the enabled metadata providers.
///
/// Unlike [`list_all_books`], this does not touch the user's library — it queries
/// official public bibliographic APIs (Open Library, Google Books) and returns
/// normalized [`BookMetadata`]. Used by the frontend's metadata lookup.
#[tauri::command]
pub async fn search_metadata(query: String) -> Result<Vec<BookMetadata>, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    let registry = MetadataRegistry::from_config(&app_config);
    registry.search(&query, 10).await.map_err(|e| e.to_string())
}

/// Resolve a single ISBN to normalized [`BookMetadata`], if any provider has it.
#[tauri::command]
pub async fn lookup_metadata_by_isbn(isbn: String) -> Result<Option<BookMetadata>, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    let registry = MetadataRegistry::from_config(&app_config);
    registry.by_isbn(&isbn).await.map_err(|e| e.to_string())
}

/// Return the raw cover-image bytes for a locally-scanned EPUB.
///
/// The frontend resolves `localcover://{book_id}` references (set by the Local
/// Files connector) through this command. It searches every configured Local
/// Files provider for the matching book id and returns the first embedded cover
/// found, or `None` if the book has no cover / can't be located. Bytes are
/// returned as a JSON number array for the frontend to wrap in a `Blob`.
#[tauri::command]
pub async fn get_local_cover(book_id: String) -> Result<Option<Vec<u8>>, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    for pc in app_config
        .providers
        .iter()
        .filter(|p| p.enabled && p.provider_type == LocalFilesProvider::ID)
    {
        let cfg: LocalFilesConfig = serde_json::from_value(pc.settings.clone()).unwrap_or_default();
        if let Some(bytes) = extract_cover(&cfg, &book_id) {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

/// Return the raw EPUB file bytes for a locally-scanned book, for the in-app
/// reader.
///
/// Searches every configured Local Files provider for the matching book id and
/// returns the file bytes. Like [`get_local_cover`], it can only read files under
/// the user's configured library folders (the id is matched against hashes of
/// discovered paths, never used to build a path), so it can't be steered outside
/// the library. An unknown id is an error.
#[tauri::command]
pub async fn get_book_file(book_id: String) -> Result<Vec<u8>, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    for pc in app_config
        .providers
        .iter()
        .filter(|p| p.enabled && p.provider_type == LocalFilesProvider::ID)
    {
        let cfg: LocalFilesConfig = serde_json::from_value(pc.settings.clone()).unwrap_or_default();
        if let Some(bytes) = read_book_file(&cfg, &book_id) {
            return Ok(bytes);
        }
    }
    Err(format!("book file not found for id {book_id}"))
}

/// Persist the reading position for a book so it can be resumed later, and — if
/// the user has opted in — mirror it to their Hardcover account.
///
/// The reader calls this on location change with the current EPUB CFI/locator and
/// percent. The local store (see [`ReadingStore`]) is the **source of truth** and
/// is written first; it always succeeds independently of any tracker.
///
/// When `book` is supplied and Hardcover sync is enabled
/// (`hardcover.sync_reading_progress`, default off), progress is *best-effort*
/// pushed to Hardcover (start → currently-reading, finish → read) via
/// [`sync_reading_progress`]. Any tracker/network error is logged and swallowed —
/// it never fails this command or the reader. See [`libro_core::sync`].
#[tauri::command]
pub async fn save_reading_progress(
    book_id: String,
    progress: Progress,
    book: Option<Book>,
    sync_state: tauri::State<'_, ReadingSyncState>,
) -> Result<(), String> {
    // 1. Source of truth: persist locally. This must always succeed on its own.
    ReadingStore::default_store()
        .save(&book_id, progress.clone())
        .map_err(|e| e.to_string())?;

    // 2. Best-effort outward sync. Failures here are swallowed by design.
    if let Some(book) = book {
        best_effort_hardcover_sync(&sync_state, &book, &progress).await;
    }
    Ok(())
}

/// Push reading progress to Hardcover if the user configured it and opted in.
/// Never returns an error — the local save already succeeded and is authoritative.
async fn best_effort_hardcover_sync(state: &ReadingSyncState, book: &Book, progress: &Progress) {
    let Ok(app_config) = config::load_config() else {
        return;
    };
    let Some(pc) = app_config
        .providers
        .iter()
        .find(|p| p.enabled && p.provider_type == HardcoverProvider::ID)
    else {
        return; // Hardcover not configured → nothing to mirror to.
    };

    let cfg: HardcoverConfig = serde_json::from_value(pc.settings.clone()).unwrap_or_default();
    let has_key = !cfg.api_key.trim().is_empty();
    let enabled = cfg.sync_reading_progress;
    let tracker = if has_key {
        Some(HardcoverProvider::new(cfg))
    } else {
        None
    };

    let outcome = sync_reading_progress(enabled, tracker.as_ref(), state, book, progress).await;
    if let SyncOutcome::Failed(e) = &outcome {
        eprintln!("hardcover reading-progress sync failed (ignored): {e}");
    }
}

/// Fetch the stored reading position for a book, if any, so the reader can resume
/// where the user left off.
#[tauri::command]
pub async fn get_reading_progress(book_id: String) -> Result<Option<Progress>, String> {
    ReadingStore::default_store()
        .get(&book_id)
        .map_err(|e| e.to_string())
}

/// Resolve a directly-playable audiobook **manifest** (all tracks on one
/// book-absolute timeline + chapters) for the in-app audio player.
///
/// `book_id` is an Audiobookshelf library-item id (the `id` of a `Book` whose
/// `source_provider_id == "audiobookshelf"`). This opens an ABS playback session
/// with the connector's auth and returns a normalized [`PlaybackManifest`]; the
/// frontend player loads each track's URL directly and auto-advances across
/// track boundaries on the unified timeline.
///
/// TODO(live): there is no ABS server in this build environment, so this path is
/// only structurally verified (the pure session→manifest mapping is unit-tested
/// in [`libro_core::providers::audiobookshelf`]). Live streaming needs a running
/// ABS instance — see `ARCHITECTURE.md`.
#[tauri::command]
pub async fn get_audiobook_stream(book_id: String) -> Result<PlaybackManifest, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;

    let mut last_err = String::from("no Audiobookshelf provider configured");
    for pc in app_config
        .providers
        .iter()
        .filter(|p| p.enabled && p.provider_type == AudiobookshelfProvider::ID)
    {
        let cfg: AudiobookshelfConfig =
            serde_json::from_value(pc.settings.clone()).unwrap_or_default();
        let provider = AudiobookshelfProvider::new(cfg);
        match provider.resolve_playback(&book_id).await {
            Ok(playback) => return Ok(playback),
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
}

/// Persist the listening (audiobook) position for a book so it can be resumed.
///
/// The audio player calls this on a throttled `timeupdate` (and on pause /
/// chapter change) with the current position in seconds + percent. The on-device
/// [`ListeningStore`] is the **source of truth** and is written first; it always
/// succeeds independently of any server.
///
/// When `book` is supplied, the book came from Audiobookshelf, and ABS
/// listening-sync is enabled (`audiobookshelf.sync_listening_progress`, default
/// off), the position is *best-effort* pushed back to the ABS server via
/// [`sync_listening_progress`]. Any network/sync error is logged and swallowed —
/// it never fails this command or the player. See [`libro_core::listening_sync`].
#[tauri::command]
pub async fn save_listening_progress(
    book_id: String,
    progress: Progress,
    book: Option<Book>,
    sync_state: tauri::State<'_, ListeningSyncState>,
) -> Result<(), String> {
    // 1. Source of truth: persist locally. This must always succeed on its own.
    ListeningStore::default_store()
        .save(&book_id, progress.clone())
        .map_err(|e| e.to_string())?;

    // 2. Best-effort outward sync to Audiobookshelf. Failures are swallowed.
    if let Some(book) = book {
        if book.source_provider_id == AudiobookshelfProvider::ID {
            // The ABS libraryItemId is the audiobook Book.id (the id
            // get_audiobook_stream opens a play session with).
            best_effort_abs_listening_sync(&sync_state, &book.id, &progress).await;
        }
    }
    Ok(())
}

/// Push listening progress to Audiobookshelf if configured and opted in.
/// Never returns an error — the local save already succeeded and is authoritative.
async fn best_effort_abs_listening_sync(
    state: &ListeningSyncState,
    item_id: &str,
    progress: &Progress,
) {
    let Ok(app_config) = config::load_config() else {
        return;
    };
    let Some(pc) = app_config
        .providers
        .iter()
        .find(|p| p.enabled && p.provider_type == AudiobookshelfProvider::ID)
    else {
        return; // ABS not configured → nothing to mirror to.
    };

    let cfg: AudiobookshelfConfig = serde_json::from_value(pc.settings.clone()).unwrap_or_default();
    let configured = !cfg.base_url.trim().is_empty() && !cfg.api_token.trim().is_empty();
    let enabled = cfg.sync_listening_progress;
    let tracker = if configured {
        Some(AudiobookshelfProvider::new(cfg))
    } else {
        None
    };

    let outcome =
        sync_listening_progress(enabled, tracker.as_ref(), state, item_id, progress).await;
    if let ListeningSyncOutcome::Failed(e) = &outcome {
        eprintln!("audiobookshelf listening-progress sync failed (ignored): {e}");
    }
}

/// Fetch the stored listening position for a book, if any, so the player can
/// resume where the user left off.
#[tauri::command]
pub async fn get_listening_progress(book_id: String) -> Result<Option<Progress>, String> {
    ListeningStore::default_store()
        .get(&book_id)
        .map_err(|e| e.to_string())
}

/// Pull remote reading/listening progress **down** and reconcile it against the
/// local stores, so a book advanced on another device resumes at the right place
/// here. This is the inbound counterpart to the outward `save_*_progress` syncs.
///
/// Design (mirrors the outward syncs' posture):
/// - **Opt-in:** a lane only runs when its provider is configured *and* its
///   `pull_progress` toggle is on (default off). Pulling remote state is never a
///   silent side effect of loading the library.
/// - **Lanes never cross:** ABS-sourced audiobooks reconcile against
///   Audiobookshelf (seconds/[`ListeningStore`]); other ebooks reconcile against
///   Hardcover (CFI/[`ReadingStore`]). A Hardcover-sourced book is skipped.
/// - **Bounded + failure-isolated:** remote fetches run with bounded concurrency
///   then winners are applied sequentially (the file stores share a temp path).
///   Every network/store error is swallowed into the returned report; this never
///   fails and the local store stays the source of truth.
/// - **No feedback loop:** after a pull-down we seed the listening throttle state
///   ([`ListeningSyncState::note_synced_position`]) so the reconciled position is
///   not immediately echoed back up to ABS on the next save.
///
/// TODO(live): the actual remote reads need a running ABS/Hardcover account; the
/// mapping + reconcile policy are unit-tested in `libro_core` (no network).
#[tauri::command]
pub async fn reconcile_progress(
    books: Vec<Book>,
    listening_state: tauri::State<'_, ListeningSyncState>,
    conflict_state: tauri::State<'_, ConflictState>,
) -> Result<ReconcileReport, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;

    // --- Build the ABS (audio) source, if configured + opted-in. -------------
    let mut abs_provider: Option<AudiobookshelfProvider> = None;
    let mut abs_enabled = false;
    if let Some(pc) = app_config
        .providers
        .iter()
        .find(|p| p.enabled && p.provider_type == AudiobookshelfProvider::ID)
    {
        let cfg: AudiobookshelfConfig =
            serde_json::from_value(pc.settings.clone()).unwrap_or_default();
        let configured = !cfg.base_url.trim().is_empty() && !cfg.api_token.trim().is_empty();
        if configured {
            abs_enabled = cfg.pull_progress;
            let mut p = AudiobookshelfProvider::new(cfg);
            let _ = p.authenticate(&pc.settings).await; // best-effort
            abs_provider = Some(p);
        }
    }

    // --- Build the Hardcover (reading) source, if configured + opted-in. -----
    let mut hc_provider: Option<HardcoverProvider> = None;
    let mut hc_enabled = false;
    if let Some(pc) = app_config
        .providers
        .iter()
        .find(|p| p.enabled && p.provider_type == HardcoverProvider::ID)
    {
        let cfg: HardcoverConfig = serde_json::from_value(pc.settings.clone()).unwrap_or_default();
        if !cfg.api_key.trim().is_empty() {
            hc_enabled = cfg.pull_progress;
            let mut p = HardcoverProvider::new(cfg);
            let _ = p.authenticate(&pc.settings).await; // sets user_id
            hc_provider = Some(p);
        }
    }

    let listening_store = ListeningStore::default_store();
    let reading_store = ReadingStore::default_store();

    let audio_lane = abs_provider.as_ref().map(|p| SyncLane {
        enabled: abs_enabled,
        source: p as &dyn ProgressSource,
        store: &listening_store as &dyn ProgressStoreLike,
    });
    let reading_lane = hc_provider.as_ref().map(|p| SyncLane {
        enabled: hc_enabled,
        source: p as &dyn ProgressSource,
        store: &reading_store as &dyn ProgressStoreLike,
    });

    let report = reconcile_catalog_with_policy(
        &books,
        audio_lane.as_ref(),
        reading_lane.as_ref(),
        RECONCILE_CONCURRENCY,
        app_config.conflict_resolution,
    )
    .await;
    let (report, conflicts) = report;

    // Stash any pending conflicts (manual mode only; auto mode yields none) for
    // the UI to render and resolve. Replaces the previous pass's set.
    conflict_state.replace_all(conflicts);

    // Seed the outward listening throttle for every ABS audiobook at its current
    // stored position, so a reconciled pull-down isn't bounced straight back to
    // ABS on the next save (feedback-loop avoidance). Idempotent for untouched
    // books — a subsequent save at the same position simply sees no delta.
    if audio_lane.is_some() && abs_enabled {
        for b in books.iter().filter(|b| lane_for(b) == Some(Lane::Audio)) {
            if let Ok(Some(p)) = listening_store.get(&b.id) {
                if let Some(pos) = p.position_seconds {
                    listening_state.note_synced_position(&b.id, pos, p.finished);
                }
            }
        }
    }

    Ok(report)
}

/// List the progress conflicts awaiting manual resolution (see
/// [`ConflictResolution::Manual`]). Empty in auto mode or when the last
/// reconcile pass found no genuine conflicts.
#[tauri::command]
pub fn list_progress_conflicts(
    conflict_state: tauri::State<'_, ConflictState>,
) -> Vec<ProgressConflict> {
    conflict_state.list()
}

/// Resolve one pending progress conflict by writing the chosen [`Progress`] into
/// the correct local store lane, seeding the outward-sync throttle so the
/// resolved value isn't spuriously pushed back out, and clearing it from the
/// pending set.
///
/// Returns `true` if a conflict was resolved, `false` if the id was unknown
/// (already resolved or never pending). Failure-isolated: a store write error
/// surfaces as `Err` but never affects other conflicts or library load.
#[tauri::command]
pub fn resolve_progress_conflict(
    book_id: String,
    choice: ConflictChoice,
    conflict_state: tauri::State<'_, ConflictState>,
    listening_state: tauri::State<'_, ListeningSyncState>,
) -> Result<bool, String> {
    let Some(conflict) = conflict_state.take(&book_id) else {
        return Ok(false);
    };

    let chosen: Progress = conflict.resolved(choice);

    match conflict.lane {
        ConflictLane::Listening => {
            let store = ListeningStore::default_store();
            store.save(&book_id, chosen.clone()).map_err(|e| e.to_string())?;
            // Anti-oscillation: seed the outward throttle at the resolved value
            // so it isn't immediately re-pushed to ABS.
            if let Some(pos) = chosen.position_seconds {
                listening_state.note_synced_position(&book_id, pos, chosen.finished);
            }
        }
        ConflictLane::Reading => {
            let store = ReadingStore::default_store();
            store.save(&book_id, chosen).map_err(|e| e.to_string())?;
        }
    }

    Ok(true)
}


/// A minimal, frontend-facing view of an installed plugin (for a "Plugins"
/// settings surface). Derived from the validated manifest — never exposes the
/// user's secret config values.
#[derive(Debug, serde::Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub plugin_api_version: u32,
    /// Raw capability bits (see [`libro_core::providers::ProviderCapabilities`]).
    pub capabilities: u32,
    /// Domains this plugin is sandboxed to (its only permitted network hosts).
    pub allowed_domains: Vec<String>,
}

/// List the installed plugins discovered in the on-device plugins directory.
///
/// Read-only: this reflects what the loader validated, so the UI can show which
/// connectors are available and what each is permitted to reach.
#[tauri::command]
pub async fn list_plugins() -> Result<Vec<PluginInfo>, String> {
    let registry = load_plugins(&config::data_dir().join("plugins"));
    Ok(registry
        .iter()
        .map(|p| PluginInfo {
            id: p.manifest.id.clone(),
            name: p.manifest.name.clone(),
            version: p.manifest.version.clone(),
            author: p.manifest.author.clone(),
            plugin_api_version: p.manifest.plugin_api_version,
            capabilities: p.manifest.capability_bits().bits(),
            allowed_domains: p.manifest.permissions.allowed_domains.clone(),
        })
        .collect())
}


// ---------------------------------------------------------------------------
// Send-to-Kindle (Amazon official personal-documents email flow)
// ---------------------------------------------------------------------------

/// Turn a book title into a safe `*.epub` attachment filename. Amazon uses the
/// attachment filename as the document title in the user's Kindle library.
fn kindle_filename_for(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    let stem = if trimmed.is_empty() { "book" } else { trimmed };
    format!("{stem}.epub")
}

/// Whether Send-to-Kindle is configured, so the frontend can show/hide the
/// "Send to Kindle" affordance without ever handling the SMTP secret.
#[tauri::command]
pub async fn kindle_configured() -> Result<bool, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    Ok(kindle_is_configured(&app_config.kindle))
}

/// Return the current Send-to-Kindle settings **with the SMTP password blanked**
/// so the secret is never echoed back to the UI.
#[tauri::command]
pub async fn get_kindle_config() -> Result<KindleConfig, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    let mut cfg = app_config.kindle;
    cfg.smtp_password = String::new();
    Ok(cfg)
}

/// Persist Send-to-Kindle settings into the (encrypted) app config.
///
/// A blank `smtp_password` means "keep the existing one" — the UI masks the
/// field and sends an empty string when the user didn't retype it, so we never
/// clobber a stored secret with a blank.
///
/// TODO(security): real persistence rides the same config-crypto boundary as the
/// rest of [`AppConfig`] (see `libro_core::config::save_config`).
#[tauri::command]
pub async fn save_kindle_config(config_in: KindleConfig) -> Result<(), String> {
    let mut app_config = config::load_config().map_err(|e| e.to_string())?;
    let mut incoming = config_in;
    if incoming.smtp_password.is_empty() {
        incoming.smtp_password = app_config.kindle.smtp_password.clone();
    }
    app_config.kindle = incoming;
    config::save_config(&app_config).map_err(|e| e.to_string())
}

/// Email the user's own local DRM-free EPUB to their `@kindle.com` address via
/// Amazon's official personal-documents flow.
///
/// Resolves the EPUB bytes through the same library-scoped Local Files path as
/// [`get_book_file`] (the id is matched against hashes of discovered paths, never
/// used to build a path), enforces the ~50 MB attachment cap, builds the MIME
/// message with the pure [`libro_core::kindle::build_kindle_message`] helper, and
/// sends it over the user's own SMTP account.
///
/// Unlike the background progress syncs, this is a **user action**, so it returns
/// a typed [`SendOutcome`] the UI surfaces as success or a clear failure — it
/// does not silently swallow errors.
///
/// TODO(live): sending needs the user's real SMTP account + an Amazon-approved
/// sender address; the message building, validators, size guard, and orchestration
/// are unit-tested with a fake sender (no network) in `libro_core::kindle`.
#[tauri::command]
pub async fn send_to_kindle(book_id: String, title: String) -> Result<SendOutcome, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    let cfg = &app_config.kindle;

    if !kindle_is_configured(cfg) {
        return Ok(SendOutcome::NotConfigured);
    }

    // Resolve the local EPUB bytes (library-scoped; unknown id → not found).
    let mut bytes: Option<Vec<u8>> = None;
    for pc in app_config
        .providers
        .iter()
        .filter(|p| p.enabled && p.provider_type == LocalFilesProvider::ID)
    {
        let lf: LocalFilesConfig = serde_json::from_value(pc.settings.clone()).unwrap_or_default();
        if let Some(b) = read_book_file(&lf, &book_id) {
            bytes = Some(b);
            break;
        }
    }
    let Some(bytes) = bytes else {
        return Ok(SendOutcome::SendFailed {
            reason: format!("no local EPUB found for id {book_id}"),
        });
    };

    let sender = match SmtpKindleSender::from_config(cfg) {
        Ok(s) => s,
        Err(reason) => return Ok(SendOutcome::SendFailed { reason }),
    };

    let filename = kindle_filename_for(&title);
    let subject = if title.trim().is_empty() {
        "Libro document".to_string()
    } else {
        title
    };
    Ok(send_epub_to_kindle(cfg, &subject, &filename, &bytes, &sender).await)
}
