//! Tauri command surface exposed to the frontend.

use libro_core::config::{self, AppConfig, ListeningStore, ProviderConfig, ReadingStore};
use libro_core::metadata::{enrich_catalog, BookMetadata, EnrichOptions, MetadataRegistry};
use libro_core::models::{Book, Progress};
use libro_core::providers::audiobookshelf::{
    AudioPlayback, AudiobookshelfConfig, AudiobookshelfProvider,
};
use libro_core::providers::hardcover::{HardcoverConfig, HardcoverProvider};
use libro_core::providers::lazylibrarian::{LazyLibrarianConfig, LazyLibrarianProvider};
use libro_core::providers::libby::{LibbyConfig, LibbyProvider};
use libro_core::providers::localfiles::{
    extract_cover, read_book_file, LocalFilesConfig, LocalFilesProvider,
};
use libro_core::providers::opds::{OpdsConfig, OpdsProvider};
use libro_core::providers::Provider;
use libro_core::plugins::{load_plugins, PluginProvider};
use libro_core::sync::{sync_reading_progress, ReadingSyncState, SyncOutcome};
use libro_core::listening_sync::{
    sync_listening_progress, ListeningSyncOutcome, ListeningSyncState,
};

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
                    providers.push(Box::new(PluginProvider::new(
                        loaded.manifest.clone(),
                        settings,
                    )));
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

/// Resolve a directly-playable audiobook stream + chapter list for the in-app
/// audio player.
///
/// `book_id` is an Audiobookshelf library-item id (the `id` of a `Book` whose
/// `source_provider_id == "audiobookshelf"`). This opens an ABS playback session
/// with the connector's auth and returns a normalized [`AudioPlayback`] the
/// frontend `<audio>` element can load directly.
///
/// TODO(live): there is no ABS server in this build environment, so this path is
/// only structurally verified (the pure session→playback mapping is unit-tested
/// in [`libro_core::providers::audiobookshelf`]). Live streaming needs a running
/// ABS instance — see `ARCHITECTURE.md`.
#[tauri::command]
pub async fn get_audiobook_stream(book_id: String) -> Result<AudioPlayback, String> {
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
