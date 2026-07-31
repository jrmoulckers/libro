//! LazyLibrarian connector (stub).
//!
//! [LazyLibrarian](https://gitlab.com/LazyLibrarian/LazyLibrarian) is a
//! **self-hosted** book manager the user runs themselves, exposing an official
//! REST API (`{base_url}/api?apikey={key}&cmd=...`). With Readarr retired in
//! June 2025, LazyLibrarian (and its forks) is the living request/acquisition
//! path.
//!
//! Legal boundary: Libro talks **only to the user's own LazyLibrarian instance**
//! and bundles **no indexers or content sources** of its own — the user
//! configures those inside their own LazyLibrarian (see `ARCHITECTURE.md` →
//! "Legal boundaries").
//!
//! Capabilities: [`ProviderCapabilities::CATALOG`],
//! [`ProviderCapabilities::REQUEST`], [`ProviderCapabilities::DOWNLOAD`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::Book;
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

/// Settings for the LazyLibrarian connector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LazyLibrarianConfig {
    /// Base URL of the user's LazyLibrarian instance, e.g.
    /// `http://192.168.1.10:5299`.
    pub base_url: String,
    /// API key from the LazyLibrarian settings.
    pub api_key: String,
}

/// The LazyLibrarian connector.
pub struct LazyLibrarianProvider {
    #[allow(dead_code)]
    config: LazyLibrarianConfig,
    authenticated: bool,
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl LazyLibrarianProvider {
    pub const ID: &'static str = "lazylibrarian";

    pub fn new(config: LazyLibrarianConfig) -> Self {
        Self {
            config,
            authenticated: false,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Provider for LazyLibrarianProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "LazyLibrarian"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::CATALOG
            | ProviderCapabilities::REQUEST
            | ProviderCapabilities::DOWNLOAD
    }

    async fn authenticate(&mut self, config: &serde_json::Value) -> ProviderResult<()> {
        if !config.is_null() {
            self.config = serde_json::from_value(config.clone())
                .map_err(|e| ProviderError::Config(e.to_string()))?;
        }
        // TODO(phase-2): GET `{base_url}/api?apikey={api_key}&cmd=getVersion`
        // (or `cmd=help`) and confirm a non-error JSON response.
        self.authenticated = true;
        Ok(())
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        // TODO(phase-2): call the user's own LazyLibrarian REST API, e.g.
        //   GET {base_url}/api?apikey={key}&cmd=getAllBooks
        // and map each returned book into `models::Book`. Request/download will
        // be separate capability methods:
        //   cmd=addBook / cmd=queueBook (REQUEST), cmd=forceProcess (DOWNLOAD).
        // Libro bundles no indexers; the user's instance owns all sources.
        Err(ProviderError::Unsupported)
    }
}
