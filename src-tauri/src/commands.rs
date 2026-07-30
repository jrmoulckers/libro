//! Tauri command surface exposed to the frontend.

use crate::config::{self, AppConfig, ProviderConfig};
use crate::models::Book;
use crate::providers::audiobookshelf::{AudiobookshelfConfig, AudiobookshelfProvider};
use crate::providers::Provider;

/// Instantiate the set of [`Provider`]s described by an [`AppConfig`].
///
/// This is the connector registry: adding a new connector means adding one arm
/// here that maps a `provider_type` string to its constructor. Unknown or
/// disabled providers are skipped.
fn build_providers(config: &AppConfig) -> Vec<Box<dyn Provider>> {
    let mut providers: Vec<Box<dyn Provider>> = Vec::new();

    for pc in config.providers.iter().filter(|p| p.enabled) {
        match pc.provider_type.as_str() {
            AudiobookshelfProvider::ID => {
                let abs_cfg: AudiobookshelfConfig =
                    serde_json::from_value(pc.settings.clone()).unwrap_or_default();
                providers.push(Box::new(AudiobookshelfProvider::new(abs_cfg)));
            }
            other => {
                // Unknown connector type; ignore for now. A later phase may
                // surface this to the user as a config warning.
                eprintln!("libro: unknown provider_type '{other}', skipping");
            }
        }
    }

    // Skeleton convenience: with no configured providers, spin up a stub
    // Audiobookshelf connector so the end-to-end aggregation + UI wiring is
    // demonstrable. Remove once real onboarding exists.
    if providers.is_empty() {
        providers.push(Box::new(AudiobookshelfProvider::new(
            AudiobookshelfConfig::default(),
        )));
    }

    providers
}

/// Aggregate every configured provider's library into one normalized catalog.
///
/// This is the core aggregation pattern: fan out over each [`Provider`],
/// authenticate, pull its library, and merge the results. Per-provider failures
/// are logged and skipped so one broken connector can't sink the whole catalog.
#[tauri::command]
pub async fn list_all_books() -> Result<Vec<Book>, String> {
    let app_config = config::load_config().map_err(|e| e.to_string())?;
    let mut providers = build_providers(&app_config);

    // Look up each provider's stored settings blob by type for authentication.
    let settings_for = |provider_id: &str| -> serde_json::Value {
        app_config
            .providers
            .iter()
            .find(|p: &&ProviderConfig| p.provider_type == provider_id)
            .map(|p| p.settings.clone())
            .unwrap_or(serde_json::Value::Null)
    };

    let mut merged: Vec<Book> = Vec::new();
    for provider in providers.iter_mut() {
        let provider_id = provider.id().to_string();
        if let Err(e) = provider.authenticate(&settings_for(&provider_id)).await {
            eprintln!("libro: provider '{provider_id}' failed to authenticate: {e}");
            continue;
        }
        match provider.list_library().await {
            Ok(mut books) => merged.append(&mut books),
            Err(e) => eprintln!("libro: provider '{provider_id}' list_library failed: {e}"),
        }
    }

    Ok(merged)
}
