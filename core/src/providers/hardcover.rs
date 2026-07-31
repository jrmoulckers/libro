//! Hardcover connector (stub).
//!
//! [Hardcover](https://hardcover.app/) is a social reading-tracker with an
//! **official public GraphQL API** at `https://api.hardcover.app/v1/graphql`,
//! authenticated with a user-supplied API key sent as `Authorization: Bearer
//! {key}`. Because this is an official, documented API used with the user's own
//! key, it is a legitimate integration (see `ARCHITECTURE.md` → "Legal
//! boundaries"). With Goodreads' API retired, Hardcover is Libro's reading-
//! tracker path.
//!
//! Capabilities: [`ProviderCapabilities::PROGRESS_SYNC`] only — reading status,
//! ratings, and shelves. Hardcover is **not** the user's library-of-record, so
//! it advertises neither `CATALOG` nor `HOLDS`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::Book;
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

/// The official Hardcover GraphQL endpoint.
pub const HARDCOVER_GRAPHQL_ENDPOINT: &str = "https://api.hardcover.app/v1/graphql";

/// Settings for the Hardcover connector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HardcoverConfig {
    /// User-supplied API key (from the Hardcover account settings).
    ///
    /// Sent as `Authorization: Bearer {api_key}`.
    pub api_key: String,
}

/// The Hardcover connector.
pub struct HardcoverProvider {
    #[allow(dead_code)]
    config: HardcoverConfig,
    authenticated: bool,
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl HardcoverProvider {
    pub const ID: &'static str = "hardcover";

    pub fn new(config: HardcoverConfig) -> Self {
        Self {
            config,
            authenticated: false,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Provider for HardcoverProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "Hardcover"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Reading-tracker only: read/write reading status, ratings, shelves.
        ProviderCapabilities::PROGRESS_SYNC
    }

    async fn authenticate(&mut self, config: &serde_json::Value) -> ProviderResult<()> {
        if !config.is_null() {
            self.config = serde_json::from_value(config.clone())
                .map_err(|e| ProviderError::Config(e.to_string()))?;
        }
        // TODO(phase-2): POST a GraphQL `{ me { id username } }` query to
        // HARDCOVER_GRAPHQL_ENDPOINT with `Authorization: Bearer {api_key}` and
        // confirm a non-error response.
        self.authenticated = true;
        Ok(())
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        // Hardcover does not advertise CATALOG; it is a progress/tracking source.
        // TODO(phase-2): implement progress-sync methods (read/write reading
        // status, ratings, shelves) via GraphQL queries/mutations, e.g.
        //   query { me { user_books { book { title contributions { author { name } } } status rating } } }
        // These will feed `Book.progress` / shelf metadata rather than a library
        // listing, so `list_library` intentionally returns empty.
        Err(ProviderError::Unsupported)
    }
}
