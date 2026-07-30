//! The connector/plugin system — a first-class abstraction in Libro.
//!
//! A [`Provider`] is Libro's contract for talking to one external source of
//! books/audiobooks (Audiobookshelf, a public library via OverDrive, StoryGraph,
//! Open Library, a local folder, …). Everything the app can do with a backend is
//! expressed through this trait, which lets the aggregation layer treat every
//! source uniformly and makes adding a new connector a matter of implementing
//! one trait.
//!
//! This is a pure-client design: a `Provider` implementation talks *directly* to
//! the remote API from the user's device. There is no Libro server in the middle.

pub mod audiobookshelf;

use async_trait::async_trait;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::models::Book;

/// Error type surfaced by connector operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider is not authenticated")]
    NotAuthenticated,
    #[error("operation not supported by this provider")]
    Unsupported,
    #[error("configuration error: {0}")]
    Config(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected error: {0}")]
    Other(String),
}

/// Result alias for provider operations.
pub type ProviderResult<T> = Result<T, ProviderError>;

bitflags! {
    /// The set of features a connector supports.
    ///
    /// Capabilities are advertised up-front so the UI can enable/disable actions
    /// per provider without probing. New connectors declare exactly what they can
    /// do; the aggregation layer never assumes a capability is present.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ProviderCapabilities: u32 {
        /// Can enumerate the user's library ([`Provider::list_library`]).
        const CATALOG        = 0b0000_0001;
        /// Can place/track holds (e.g. library systems).
        const HOLDS          = 0b0000_0010;
        /// Can request/acquire a title not yet owned.
        const REQUEST        = 0b0000_0100;
        /// Can download the underlying file(s).
        const DOWNLOAD       = 0b0000_1000;
        /// Can push a title to a Kindle (typically via Send-to-Kindle email).
        const SEND_TO_KINDLE = 0b0001_0000;
        /// Can read and/or write reading/listening progress.
        const PROGRESS_SYNC  = 0b0010_0000;
    }
}

// Serialize capabilities as their raw bit value so the frontend can reason about
// them (a later phase may expose a richer, named representation).
impl Serialize for ProviderCapabilities {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.bits())
    }
}

impl<'de> Deserialize<'de> for ProviderCapabilities {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bits = u32::deserialize(deserializer)?;
        Ok(ProviderCapabilities::from_bits_truncate(bits))
    }
}

/// The connector contract.
///
/// Implement this trait to add a new source to Libro. Methods are `async`
/// because real connectors perform network I/O; [`async_trait`] is used so the
/// trait remains object-safe and can be stored as `Box<dyn Provider>` in the
/// provider registry.
///
/// # Adding a connector
/// 1. Define a config struct (deserializable from the stored [`crate::config`]).
/// 2. Implement [`Provider`], advertising the right [`ProviderCapabilities`].
/// 3. Register it in the provider registry used by `list_all_books`.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable, machine-readable identifier for this connector *type*
    /// (e.g. `"audiobookshelf"`). Used as [`Book::source_provider_id`].
    fn id(&self) -> &str;

    /// Human-friendly name for display in the UI (e.g. `"Audiobookshelf"`).
    fn display_name(&self) -> &str;

    /// What this connector can do. See [`ProviderCapabilities`].
    fn capabilities(&self) -> ProviderCapabilities;

    /// Establish/verify credentials against the remote service.
    ///
    /// `config` is the provider-specific settings blob (already decrypted by the
    /// [`crate::config`] layer). Implementations should validate the connection
    /// here so later calls can assume they are authenticated.
    async fn authenticate(&mut self, config: &serde_json::Value) -> ProviderResult<()>;

    /// Enumerate the user's library as normalized [`Book`]s.
    ///
    /// Requires [`ProviderCapabilities::CATALOG`].
    async fn list_library(&self) -> ProviderResult<Vec<Book>>;
}
