//! The plugin SDK — third-party connectors without forking core.
//!
//! Phase 3 of the roadmap. A **plugin** lets a user add a new library source by
//! dropping a small, declarative manifest into their plugins directory — no
//! recompiling Libro, no shipping native code.
//!
//! # Why declarative manifests (and not WASM) for v1
//!
//! Three mechanisms were considered: (a) a WASM runtime (Extism/wasmtime),
//! (b) this declarative-manifest engine, (c) subprocess/JSON-RPC. The choice is
//! driven by Libro's hard constraints:
//!
//! * **Mobile is a first-class target.** iOS forbids JIT compilation, and the
//!   default Extism/wasmtime backend is a Cranelift JIT — a non-starter on iOS.
//!   Subprocess spawning is likewise unavailable in the iOS/Android sandboxes.
//! * **Sandboxing.** A plugin must only reach *explicitly declared* domains,
//!   with no ambient filesystem or network. A declarative manifest is inert data
//!   — there is no arbitrary code to escape the sandbox — so the host trivially
//!   mediates every network call. This is a *tighter* boundary than sandboxing
//!   arbitrary WASM.
//! * **Build/footprint.** Extism pulls ~300 transitive crates (wasmtime +
//!   cranelift), inflating the already-constrained `x86_64-pc-windows-gnu`
//!   cdylib link and the security-audit surface, for no benefit to the common
//!   case (mapping a REST/JSON catalog into [`Book`]s).
//!
//! So **v1 ships this declarative engine**: a pure-Rust interpreter over a JSON
//! manifest that describes a REST endpoint and a field→[`Book`] mapping. A WASM
//! runtime (for plugins that need real logic beyond declarative mapping) is a
//! documented future step — see `ARCHITECTURE.md`.
//!
//! # Security boundary
//!
//! Plugins honor the **same** legal/security rules as native connectors: only
//! the user's own services and official/public APIs, network restricted to the
//! manifest's `allowed_domains`, and **no** bundled scrapers/indexers or DRM
//! circumvention. The loader rejects malformed or over-broad manifests; the
//! [`engine`] denies any request whose host is not on the allowlist.
//!
//! [`Book`]: crate::models::Book

pub mod engine;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::models::MediaType;
use crate::providers::ProviderCapabilities;

pub use engine::PluginProvider;

/// The plugin API version this build implements.
///
/// A manifest must declare a matching [`PluginManifest::plugin_api_version`]; the
/// loader rejects mismatches so an incompatible manifest fails fast rather than
/// misbehaving. Bump this on any breaking change to the manifest schema.
pub const PLUGIN_API_VERSION: u32 = 1;

/// Errors from loading, validating, or running a plugin.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("plugin manifest parse error: {0}")]
    Parse(String),
    #[error("invalid plugin manifest: {0}")]
    Invalid(String),
    #[error("plugin io error: {0}")]
    Io(String),
}

/// A capability a plugin may request, mirroring [`ProviderCapabilities`] but in a
/// human-authorable, forward-compatible string form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    Catalog,
    Holds,
    Request,
    Download,
    SendToKindle,
    ProgressSync,
}

impl PluginCapability {
    fn bit(self) -> ProviderCapabilities {
        match self {
            PluginCapability::Catalog => ProviderCapabilities::CATALOG,
            PluginCapability::Holds => ProviderCapabilities::HOLDS,
            PluginCapability::Request => ProviderCapabilities::REQUEST,
            PluginCapability::Download => ProviderCapabilities::DOWNLOAD,
            PluginCapability::SendToKindle => ProviderCapabilities::SEND_TO_KINDLE,
            PluginCapability::ProgressSync => ProviderCapabilities::PROGRESS_SYNC,
        }
    }
}

/// The kind of a user-filled config field, so a UI can render the right control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    Text,
    /// Sensitive value (e.g. an API key) — the UI should mask it.
    Secret,
    Url,
}

impl Default for ConfigFieldType {
    fn default() -> Self {
        ConfigFieldType::Text
    }
}

/// One user-filled configuration field the plugin needs (e.g. `base_url`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    /// The key used to interpolate this value into requests (e.g. `base_url`).
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "type")]
    pub field_type: ConfigFieldType,
    #[serde(default)]
    pub required: bool,
}

/// Network permissions a plugin is granted. The host denies any request whose
/// host is not covered by [`Self::allowed_domains`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginPermissions {
    /// Allowlisted domains. A host matches a domain if it equals it or is a
    /// subdomain of it (e.g. `api.example.com` matches `example.com`).
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

/// HTTP method for a catalog request. v1 supports `GET`; `POST` is a TODO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
}

impl Default for HttpMethod {
    fn default() -> Self {
        HttpMethod::Get
    }
}

/// A templated HTTP request. `{key}` tokens in `url`, `headers`, and `query` are
/// interpolated from the user's config values at run time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSpec {
    #[serde(default)]
    pub method: HttpMethod,
    /// URL template, e.g. `"{base_url}/api/books"`.
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
}

/// How to map each JSON response item onto a normalized [`Book`]. Values are
/// dotted paths into a response item (e.g. `"series.name"`).
///
/// [`Book`]: crate::models::Book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMap {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub authors: Option<String>,
    #[serde(default)]
    pub series: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
}

/// The catalog specification: how to fetch the library and map it to [`Book`]s.
///
/// [`Book`]: crate::models::Book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSpec {
    pub request: RequestSpec,
    /// Dotted path to the array of items in the response (e.g. `"results"`).
    /// Empty means the response body is itself the array.
    #[serde(default)]
    pub items_path: String,
    #[serde(default = "default_media_type")]
    pub media_type: MediaType,
    pub fields: FieldMap,
}

fn default_media_type() -> MediaType {
    MediaType::Ebook
}

/// A plugin manifest — the complete, declarative description of a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Stable machine id, used as the provider type + `Book::source_provider_id`.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    /// Must equal [`PLUGIN_API_VERSION`] for this build to load the plugin.
    pub plugin_api_version: u32,
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
    #[serde(default)]
    pub permissions: PluginPermissions,
    #[serde(default)]
    pub config_schema: Vec<ConfigField>,
    pub catalog: CatalogSpec,
}

impl PluginManifest {
    /// Parse a manifest from JSON bytes (does not validate — call [`Self::validate`]).
    pub fn from_json(bytes: &[u8]) -> Result<Self, PluginError> {
        serde_json::from_slice(bytes).map_err(|e| PluginError::Parse(e.to_string()))
    }

    /// The requested capabilities folded into a [`ProviderCapabilities`] bitset.
    pub fn capability_bits(&self) -> ProviderCapabilities {
        self.capabilities
            .iter()
            .fold(ProviderCapabilities::empty(), |acc, c| acc | c.bit())
    }

    /// Validate the manifest, rejecting malformed or over-broad definitions.
    ///
    /// Enforced invariants:
    /// * `plugin_api_version` matches this build,
    /// * `id`/`name` are present and `id` has no whitespace,
    /// * the plugin requests `catalog` (the only capability v1's engine serves),
    /// * at least one `allowed_domains` entry, none wildcard/over-broad,
    /// * a non-empty request URL and `id`/`title` field mappings.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.plugin_api_version != PLUGIN_API_VERSION {
            return Err(PluginError::Invalid(format!(
                "unsupported plugin_api_version {} (this build supports {})",
                self.plugin_api_version, PLUGIN_API_VERSION
            )));
        }
        if self.id.trim().is_empty() {
            return Err(PluginError::Invalid("id is empty".into()));
        }
        if self.id.contains(char::is_whitespace) {
            return Err(PluginError::Invalid("id must not contain whitespace".into()));
        }
        if self.name.trim().is_empty() {
            return Err(PluginError::Invalid("name is empty".into()));
        }
        if !self.capabilities.contains(&PluginCapability::Catalog) {
            return Err(PluginError::Invalid(
                "plugin must request the 'catalog' capability (the only one v1 serves)".into(),
            ));
        }
        if self.permissions.allowed_domains.is_empty() {
            return Err(PluginError::Invalid(
                "permissions.allowed_domains must list at least one domain".into(),
            ));
        }
        for d in &self.permissions.allowed_domains {
            validate_domain(d)?;
        }
        if self.catalog.request.url.trim().is_empty() {
            return Err(PluginError::Invalid("catalog.request.url is empty".into()));
        }
        if self.catalog.fields.id.trim().is_empty() || self.catalog.fields.title.trim().is_empty() {
            return Err(PluginError::Invalid(
                "catalog.fields.id and .title are required".into(),
            ));
        }
        Ok(())
    }
}

/// Reject over-broad or malformed allowlist entries: no wildcards, no scheme,
/// no path, no port — just a bare host like `api.example.com`.
fn validate_domain(domain: &str) -> Result<(), PluginError> {
    let d = domain.trim();
    if d.is_empty() {
        return Err(PluginError::Invalid("empty allowed domain".into()));
    }
    if d.contains('*') {
        return Err(PluginError::Invalid(format!(
            "over-broad allowed domain '{d}' (wildcards are not permitted)"
        )));
    }
    if d.contains("://") || d.contains('/') || d.contains(':') {
        return Err(PluginError::Invalid(format!(
            "allowed domain '{d}' must be a bare host (no scheme, path, or port)"
        )));
    }
    if !d.contains('.') {
        return Err(PluginError::Invalid(format!(
            "allowed domain '{d}' is not a valid host"
        )));
    }
    Ok(())
}

/// A plugin that was discovered, parsed, and validated from disk.
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    /// The manifest file this plugin was loaded from.
    pub source_path: PathBuf,
}

/// The host-side registry of installed plugins.
///
/// Built by scanning the plugins directory once ([`load_plugins`]); the provider
/// registry (`build_providers`) consults it to instantiate a [`PluginProvider`]
/// for each configured plugin id, alongside the native connectors.
#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
}

impl PluginRegistry {
    /// An empty registry (e.g. when no plugins directory exists).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a registry directly from already-loaded plugins (used by tests).
    pub fn from_plugins(plugins: Vec<LoadedPlugin>) -> Self {
        Self { plugins }
    }

    /// Look up a loaded plugin by its manifest id.
    pub fn get(&self, id: &str) -> Option<&LoadedPlugin> {
        self.plugins.iter().find(|p| p.manifest.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &LoadedPlugin> {
        self.plugins.iter()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

/// Discover and load every valid plugin manifest (`*.json`) in `dir`.
///
/// Robust by design: a missing directory yields an empty registry (not an
/// error), and a single malformed/invalid/over-broad manifest is skipped with a
/// log — one bad plugin can never abort discovery or crash the app.
pub fn load_plugins(dir: &Path) -> PluginRegistry {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return PluginRegistry::empty(), // no plugins dir → no plugins.
    };

    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match load_one(&path) {
            Ok(loaded) => plugins.push(loaded),
            Err(e) => eprintln!("libro: skipping plugin {}: {e}", path.display()),
        }
    }
    PluginRegistry::from_plugins(plugins)
}

/// Load, parse, and validate a single manifest file.
pub fn load_one(path: &Path) -> Result<LoadedPlugin, PluginError> {
    let bytes = fs::read(path).map_err(|e| PluginError::Io(e.to_string()))?;
    let manifest = PluginManifest::from_json(&bytes)?;
    manifest.validate()?;
    Ok(LoadedPlugin {
        manifest,
        source_path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest_json() -> &'static str {
        r#"{
          "id": "example-rest-catalog",
          "name": "Example REST Catalog",
          "version": "1.0.0",
          "author": "Libro contributors",
          "plugin_api_version": 1,
          "capabilities": ["catalog"],
          "permissions": { "allowed_domains": ["api.example-books.test"] },
          "config_schema": [
            { "key": "base_url", "label": "Server URL", "type": "url", "required": true }
          ],
          "catalog": {
            "request": { "method": "GET", "url": "{base_url}/api/books" },
            "items_path": "results",
            "media_type": "Ebook",
            "fields": { "id": "id", "title": "title", "authors": "authors" }
          }
        }"#
    }

    #[test]
    fn parses_and_validates_a_good_manifest() {
        let m = PluginManifest::from_json(valid_manifest_json().as_bytes()).unwrap();
        m.validate().unwrap();
        assert_eq!(m.id, "example-rest-catalog");
        assert_eq!(m.plugin_api_version, PLUGIN_API_VERSION);
        assert!(m.capability_bits().contains(ProviderCapabilities::CATALOG));
        assert_eq!(m.config_schema[0].field_type, ConfigFieldType::Url);
    }

    #[test]
    fn rejects_wrong_api_version() {
        let json =
            valid_manifest_json().replace("\"plugin_api_version\": 1", "\"plugin_api_version\": 99");
        let m = PluginManifest::from_json(json.as_bytes()).unwrap();
        let err = m.validate().unwrap_err();
        assert!(matches!(err, PluginError::Invalid(_)));
    }

    #[test]
    fn rejects_over_broad_wildcard_domain() {
        let json = valid_manifest_json().replace("api.example-books.test", "*");
        let m = PluginManifest::from_json(json.as_bytes()).unwrap();
        assert!(matches!(m.validate(), Err(PluginError::Invalid(_))));
    }

    #[test]
    fn rejects_domain_with_scheme_or_path() {
        for bad in ["https://api.example-books.test", "api.example-books.test/x"] {
            let json = valid_manifest_json().replace("api.example-books.test", bad);
            let m = PluginManifest::from_json(json.as_bytes()).unwrap();
            assert!(
                matches!(m.validate(), Err(PluginError::Invalid(_))),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn rejects_empty_allowed_domains() {
        let json = valid_manifest_json().replace("[\"api.example-books.test\"]", "[]");
        let m = PluginManifest::from_json(json.as_bytes()).unwrap();
        assert!(matches!(m.validate(), Err(PluginError::Invalid(_))));
    }

    #[test]
    fn rejects_manifest_without_catalog_capability() {
        let json = valid_manifest_json().replace("[\"catalog\"]", "[\"download\"]");
        let m = PluginManifest::from_json(json.as_bytes()).unwrap();
        assert!(matches!(m.validate(), Err(PluginError::Invalid(_))));
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        assert!(matches!(
            PluginManifest::from_json(b"{ not json"),
            Err(PluginError::Parse(_))
        ));
    }

    #[test]
    fn load_plugins_discovers_valid_and_skips_invalid() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("good.json"), valid_manifest_json()).unwrap();
        // An invalid manifest (bad api version) must be skipped, not fatal.
        let bad = valid_manifest_json()
            .replace("\"plugin_api_version\": 1", "\"plugin_api_version\": 99");
        fs::write(dir.path().join("bad.json"), bad).unwrap();
        // A non-json file is ignored.
        fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();

        let reg = load_plugins(dir.path());
        assert_eq!(reg.len(), 1);
        assert!(reg.get("example-rest-catalog").is_some());
    }

    #[test]
    fn load_plugins_on_missing_dir_is_empty_not_error() {
        let reg = load_plugins(Path::new("C:/definitely/not/here/libro-plugins"));
        assert!(reg.is_empty());
    }
}
