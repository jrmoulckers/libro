//! Local, encrypted-at-rest application configuration.
//!
//! Libro is a **pure client**: all configuration lives on the user's device.
//! There is no config server. In the target design the config is:
//!   * stored encrypted at rest (key held in the OS keychain), and
//!   * recoverable/syncable across devices via a user-controlled, encrypted
//!     backup blob using a Signal-style device-to-device model.
//!
//! This module defines the config *types* and the save/load *boundary*. The
//! actual cryptography and keychain integration are deliberately **not**
//! implemented yet — see the TODOs on [`load_config`]/[`save_config`].

use serde::{Deserialize, Serialize};

pub mod listening;
pub mod reading;
pub use listening::ListeningStore;
pub use reading::ReadingStore;

/// A single configured provider instance.
///
/// `provider_type` selects the connector (e.g. `"audiobookshelf"`) and
/// `settings` is that connector's own config blob (e.g.
/// [`crate::providers::audiobookshelf::AudiobookshelfConfig`]). Keeping settings
/// as opaque JSON here means adding a new connector never requires touching this
/// enum-free config shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// A stable, user-facing instance id (lets a user configure two accounts of
    /// the same provider type).
    pub instance_id: String,
    /// The connector type id, matching [`crate::providers::Provider::id`].
    pub provider_type: String,
    /// Whether this provider participates in aggregation.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Connector-specific settings blob (decrypted in memory).
    #[serde(default)]
    pub settings: serde_json::Value,
}

fn default_true() -> bool {
    true
}

/// Configuration for the metadata-enrichment layer (see [`crate::metadata`]).
///
/// Metadata providers are *not* library `Provider`s — they enrich normalized
/// books rather than list a user's owned catalog — so their settings live here
/// rather than in [`ProviderConfig`]. Open Library needs no auth; Google Books
/// works anonymously but an optional API key raises rate limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataConfig {
    /// Optional Google Books API key (raises the anonymous rate limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_books_api_key: Option<String>,
    /// Whether the aggregation pipeline auto-enriches catalog books with missing
    /// fields (cover, series, description, identifiers) from the metadata
    /// providers. Defaults to `true`; set `false` to skip the enrichment pass.
    #[serde(default = "default_true")]
    pub enrich_catalog: bool,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        Self {
            google_books_api_key: None,
            enrich_catalog: true,
        }
    }
}

/// Top-level application configuration persisted on the device.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Schema version, to allow forward migrations of the on-disk format.
    #[serde(default)]
    pub version: u32,
    /// All configured providers.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Metadata-enrichment settings.
    #[serde(default)]
    pub metadata: MetadataConfig,
}

impl AppConfig {
    pub const CURRENT_VERSION: u32 = 1;
}

/// Errors from loading/saving configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(String),
    #[error("decryption failed")]
    Decryption,
}

/// Load and decrypt the application configuration from local storage.
///
/// TODO(security): implement the real pipeline:
///   1. Read the encrypted config blob from the platform config dir.
///   2. Fetch the data-encryption key from the OS keychain
///      (Keychain / Credential Manager / libsecret; mobile secure enclave).
///   3. Decrypt (AEAD, e.g. XChaCha20-Poly1305) and deserialize.
///   4. Reconcile with the Signal-style encrypted backup blob when syncing
///      across devices.
///
/// For the skeleton this returns an empty, default config so the rest of the app
/// can run without any stored secrets.
pub fn load_config() -> Result<AppConfig, ConfigError> {
    Ok(AppConfig {
        version: AppConfig::CURRENT_VERSION,
        providers: Vec::new(),
        ..Default::default()
    })
}

/// Encrypt and persist the application configuration to local storage.
///
/// TODO(security): mirror [`load_config`] — serialize, encrypt with the
/// keychain-held key, write atomically to the platform config dir, and update
/// the user-controlled encrypted backup blob for device-to-device recovery.
///
/// For the skeleton this is a no-op that only validates the config serializes.
pub fn save_config(config: &AppConfig) -> Result<(), ConfigError> {
    let _serialized = serde_json::to_vec(config)?;
    // TODO(security): encrypt `_serialized` and write it to disk.
    Ok(())
}
