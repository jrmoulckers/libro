//! Tauri command surface exposed to the frontend.

use libro_core::config::{self, AppConfig, ProviderConfig};
use libro_core::metadata::{enrich_catalog, BookMetadata, EnrichOptions, MetadataRegistry};
use libro_core::models::Book;
use libro_core::providers::audiobookshelf::{AudiobookshelfConfig, AudiobookshelfProvider};
use libro_core::providers::hardcover::{HardcoverConfig, HardcoverProvider};
use libro_core::providers::lazylibrarian::{LazyLibrarianConfig, LazyLibrarianProvider};
use libro_core::providers::libby::{LibbyConfig, LibbyProvider};
use libro_core::providers::localfiles::{extract_cover, LocalFilesConfig, LocalFilesProvider};
use libro_core::providers::opds::{OpdsConfig, OpdsProvider};
use libro_core::providers::Provider;

/// Instantiate the set of [`Provider`]s described by an [`AppConfig`].
///
/// This is the connector registry: adding a new connector means adding one arm
/// here that maps a `provider_type` string to its constructor. Unknown or
/// disabled providers are skipped. With no configured providers this returns an
/// empty list (the frontend then shows the empty/onboarding state).
fn build_providers(config: &AppConfig) -> Vec<Box<dyn Provider>> {
    let mut providers: Vec<Box<dyn Provider>> = Vec::new();

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
                // Unknown connector type; ignore for now. A later phase may
                // surface this to the user as a config warning.
                eprintln!("libro: unknown provider_type '{other}', skipping");
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

