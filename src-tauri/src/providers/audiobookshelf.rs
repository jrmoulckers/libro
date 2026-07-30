//! Stub Audiobookshelf connector.
//!
//! [Audiobookshelf](https://www.audiobookshelf.org/) is a self-hosted audiobook
//! and podcast server with a documented REST API. This module defines the config
//! shape and wires up the [`Provider`] trait, but the actual HTTP calls are left
//! as TODOs for phase 1.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::Book;
use crate::providers::{Provider, ProviderCapabilities, ProviderResult};

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
}

impl AudiobookshelfProvider {
    pub const ID: &'static str = "audiobookshelf";

    pub fn new(config: AudiobookshelfConfig) -> Self {
        Self {
            config,
            authenticated: false,
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
                .map_err(|e| crate::providers::ProviderError::Config(e.to_string()))?;
        }
        // TODO(phase-1): validate credentials, e.g. GET `{base_url}/api/me`
        // with `Authorization: Bearer {api_token}` and confirm a 200 response.
        self.authenticated = true;
        Ok(())
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        // TODO(phase-1): call the real Audiobookshelf REST API.
        //   1. GET `{base_url}/api/libraries` to discover library ids.
        //   2. GET `{base_url}/api/libraries/{id}/items` (paginated) for items.
        //   3. Map each `LibraryItem` (media.metadata.title/authors/series, id,
        //      cover path) into `models::Book`, and map media progress via
        //      `{base_url}/api/me` progress payloads.
        // See: https://api.audiobookshelf.org/
        //
        // For the skeleton we return a single mock item so the end-to-end
        // aggregation + UI wiring can be exercised without a live server.
        let _ = &self.config; // silence unused warning until wired up.

        let mut mock = Book::new(
            "abs-mock-1",
            "The Fellowship of the Ring (mock)",
            crate::models::MediaType::Audiobook,
            Self::ID,
        );
        mock.authors = vec!["J. R. R. Tolkien".to_string()];
        mock.series = Some("The Lord of the Rings".to_string());
        Ok(vec![mock])
    }
}
