//! The declarative manifest engine — interprets a [`PluginManifest`] as a
//! [`Provider`].
//!
//! [`PluginProvider`] turns a manifest's [`CatalogSpec`](super::CatalogSpec) into
//! live [`Book`]s: it interpolates the user's config into the request template,
//! **enforces the network sandbox** (the resolved host must be on the manifest's
//! `allowed_domains`), fetches the JSON, and maps each item onto a normalized
//! [`Book`] via the declared field paths.
//!
//! The interpolation, sandbox check, and response mapping are pure functions so
//! they can be unit-tested from fixtures with no network.

use async_trait::async_trait;
use serde_json::Value;

use crate::models::Book;
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

use super::{HttpMethod, PluginManifest};

/// A connector driven entirely by a [`PluginManifest`] (no native code).
pub struct PluginProvider {
    manifest: PluginManifest,
    /// The user's saved settings blob (config values interpolated into requests).
    config: Value,
    client: reqwest::Client,
}

impl PluginProvider {
    /// Create a provider for `manifest`, seeded with the user's stored `settings`.
    pub fn new(manifest: PluginManifest, settings: Value) -> Self {
        Self {
            manifest,
            config: settings,
            client: reqwest::Client::new(),
        }
    }

    /// Verify every `required` config field is present and non-empty.
    fn check_required_config(&self) -> ProviderResult<()> {
        for field in self.manifest.config_schema.iter().filter(|f| f.required) {
            let present = self
                .config
                .get(&field.key)
                .and_then(value_to_string)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !present {
                return Err(ProviderError::Config(format!(
                    "missing required config '{}'",
                    field.key
                )));
            }
        }
        Ok(())
    }
}

/// A fully-resolved (interpolated + sandbox-checked) HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[async_trait]
impl Provider for PluginProvider {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    fn display_name(&self) -> &str {
        &self.manifest.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.manifest.capability_bits()
    }

    async fn authenticate(&mut self, config: &Value) -> ProviderResult<()> {
        if !config.is_null() {
            self.config = config.clone();
        }
        self.check_required_config()
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        // v1 only serves GET catalogs.
        match self.manifest.catalog.request.method {
            HttpMethod::Get => {}
        }

        // Interpolate + enforce the network sandbox before any request goes out.
        let req = resolve_request(&self.manifest, &self.config)?;

        let mut builder = self.client.get(&req.url);
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        match resp.status().as_u16() {
            200 => {
                let json: Value = resp
                    .json()
                    .await
                    .map_err(|e| ProviderError::Other(format!("invalid JSON response: {e}")))?;
                Ok(map_response(&self.manifest, &json))
            }
            401 | 403 => Err(ProviderError::NotAuthenticated),
            other => Err(ProviderError::Api(format!("unexpected status {other}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without any network access).
// ---------------------------------------------------------------------------

/// Interpolate a `{key}` template against the config object.
///
/// Every `{key}` must resolve to a config value; an unresolved token is an error
/// (this is how required-but-empty config is caught at request-build time).
fn interpolate(template: &str, config: &Value) -> ProviderResult<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| {
            ProviderError::Config(format!("unterminated '{{' in template: {template}"))
        })?;
        let key = &after[..close];
        let value = config
            .get(key)
            .and_then(value_to_string)
            .ok_or_else(|| ProviderError::Config(format!("missing config value '{key}'")))?;
        out.push_str(&value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Build the final request: interpolate url/query/headers, append the query
/// string, then enforce the domain allowlist on the resolved URL.
pub fn resolve_request(manifest: &PluginManifest, config: &Value) -> ProviderResult<ResolvedRequest> {
    let spec = &manifest.catalog.request;
    let mut url = interpolate(&spec.url, config)?;

    if !spec.query.is_empty() {
        let mut pairs: Vec<String> = Vec::new();
        for (k, v) in &spec.query {
            let value = interpolate(v, config)?;
            pairs.push(format!("{}={}", url_encode(k), url_encode(&value)));
        }
        let sep = if url.contains('?') { '&' } else { '?' };
        url = format!("{url}{sep}{}", pairs.join("&"));
    }

    // SANDBOX: the resolved host must be covered by the manifest's allowlist.
    check_domain_allowed(&url, &manifest.permissions.allowed_domains)?;

    let mut headers = Vec::new();
    for (name, value_tmpl) in &spec.headers {
        headers.push((name.clone(), interpolate(value_tmpl, config)?));
    }

    Ok(ResolvedRequest { url, headers })
}

/// Enforce the network sandbox: the URL must be http(s) and its host must match
/// one of the allowlisted domains, else a typed error (never a panic).
pub fn check_domain_allowed(url: &str, allowed: &[String]) -> ProviderResult<()> {
    let host = host_of(url).ok_or_else(|| {
        ProviderError::Config(format!("could not parse a host from URL '{url}'"))
    })?;
    if allowed.iter().any(|d| host_matches_domain(&host, d)) {
        Ok(())
    } else {
        Err(ProviderError::Config(format!(
            "network permission denied: host '{host}' is not in allowed_domains"
        )))
    }
}

/// Extract the host from an http(s) URL, lowercased and without port.
fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // Drop any userinfo and port.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = authority.split(':').next().unwrap_or(authority);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// A host matches a domain if it equals it or is a subdomain of it.
fn host_matches_domain(host: &str, domain: &str) -> bool {
    let d = domain.trim().to_ascii_lowercase();
    if d.is_empty() {
        return false;
    }
    host == d || host.ends_with(&format!(".{d}"))
}

/// Minimal percent-encoding for query values (RFC 3986 unreserved kept as-is).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Map a JSON response to normalized [`Book`]s using the manifest's field paths.
///
/// Robust: a non-array items location or a non-object/id-less/title-less item is
/// skipped (logged), never fatal — one bad record can't sink the whole catalog.
pub fn map_response(manifest: &PluginManifest, json: &Value) -> Vec<Book> {
    let catalog = &manifest.catalog;
    let items = if catalog.items_path.trim().is_empty() {
        json.as_array().cloned().unwrap_or_default()
    } else {
        get_path(json, &catalog.items_path)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };

    let f = &catalog.fields;
    let mut books = Vec::new();
    for item in &items {
        if !item.is_object() {
            continue;
        }
        let id = get_string(item, &f.id);
        let title = get_string(item, &f.title);
        let (id, title) = match (id, title) {
            (Some(id), Some(title)) if !id.is_empty() && !title.is_empty() => (id, title),
            _ => {
                eprintln!("libro: plugin '{}' skipping item missing id/title", manifest.id);
                continue;
            }
        };

        let mut book = Book::new(id, title, catalog.media_type, &manifest.id);

        if let Some(path) = &f.authors {
            book.authors = extract_authors(item, path);
        }
        if let Some(path) = &f.series {
            book.series = get_string(item, path).filter(|s| !s.is_empty());
        }
        if let Some(path) = &f.cover_url {
            book.cover_url = get_string(item, path).filter(|s| !s.is_empty());
        }
        if let Some(path) = &f.description {
            book.description = get_string(item, path).filter(|s| !s.is_empty());
        }
        if let Some(path) = &f.isbn {
            if let Some(isbn) = get_string(item, path).filter(|s| !s.is_empty()) {
                book.identifiers.insert("isbn".into(), isbn);
            }
        }
        books.push(book);
    }
    books
}

/// Traverse a dotted path (`a.b.c`) into a JSON value.
fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = value;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Resolve a dotted path to a scalar string (string/number/bool coerced).
fn get_string(item: &Value, path: &str) -> Option<String> {
    get_path(item, path).and_then(value_to_string)
}

/// Coerce a scalar JSON value to a string. Objects/arrays/null → `None`.
fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Extract authors: an array of scalars, or a single comma-separated string.
fn extract_authors(item: &Value, path: &str) -> Vec<String> {
    match get_path(item, path) {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(value_to_string)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaType;
    use std::path::Path;

    fn manifest() -> PluginManifest {
        let json = r#"{
          "id": "example-rest-catalog",
          "name": "Example REST Catalog",
          "plugin_api_version": 1,
          "capabilities": ["catalog"],
          "permissions": { "allowed_domains": ["api.example-books.test"] },
          "config_schema": [
            { "key": "base_url", "type": "url", "required": true },
            { "key": "api_key", "type": "secret", "required": false }
          ],
          "catalog": {
            "request": {
              "method": "GET",
              "url": "{base_url}/api/books",
              "headers": { "Authorization": "Bearer {api_key}" },
              "query": { "limit": "100" }
            },
            "items_path": "results",
            "media_type": "Ebook",
            "fields": {
              "id": "id", "title": "title", "authors": "authors",
              "series": "series.name", "cover_url": "cover",
              "description": "summary", "isbn": "identifiers.isbn_13"
            }
          }
        }"#;
        PluginManifest::from_json(json.as_bytes()).unwrap()
    }

    fn config(base: &str) -> Value {
        serde_json::json!({ "base_url": base, "api_key": "SECRET123" })
    }

    #[test]
    fn resolves_request_interpolating_config_and_query() {
        let req =
            resolve_request(&manifest(), &config("https://api.example-books.test")).unwrap();
        assert_eq!(req.url, "https://api.example-books.test/api/books?limit=100");
        assert_eq!(
            req.headers,
            vec![("Authorization".to_string(), "Bearer SECRET123".to_string())]
        );
    }

    #[test]
    fn denies_request_to_a_disallowed_domain() {
        // The user points base_url at a host NOT on the allowlist → sandbox error.
        let err = resolve_request(&manifest(), &config("https://evil.example.com")).unwrap_err();
        match err {
            ProviderError::Config(m) => assert!(m.contains("permission denied")),
            other => panic!("expected a permission error, got {other:?}"),
        }
    }

    #[test]
    fn subdomains_of_an_allowed_domain_are_permitted() {
        assert!(check_domain_allowed(
            "https://cdn.api.example-books.test/x",
            &["api.example-books.test".into()]
        )
        .is_ok());
        assert!(check_domain_allowed(
            "https://api.example-books.test.evil.com/x",
            &["api.example-books.test".into()]
        )
        .is_err());
    }

    #[test]
    fn interpolate_missing_key_is_an_error() {
        let cfg = serde_json::json!({ "base_url": "https://api.example-books.test" });
        // api_key is absent → the Authorization header can't be built.
        let err = resolve_request(&manifest(), &cfg).unwrap_err();
        assert!(matches!(err, ProviderError::Config(_)));
    }

    #[test]
    fn maps_response_items_to_books_and_skips_malformed() {
        let json = serde_json::json!({
            "results": [
                {
                    "id": "b1",
                    "title": "Sandbox Stories",
                    "authors": ["Ada Lovelace", "Alan Turing"],
                    "series": { "name": "Foundations" },
                    "cover": "https://api.example-books.test/covers/b1.jpg",
                    "summary": "A book about safe extensibility.",
                    "identifiers": { "isbn_13": "9781234567897" }
                },
                { "id": "b2" },  // missing title → skipped
                { "title": "No Id" },  // missing id → skipped
                "not-an-object"
            ]
        });
        let books = map_response(&manifest(), &json);
        assert_eq!(books.len(), 1);
        let b = &books[0];
        assert_eq!(b.id, "b1");
        assert_eq!(b.title, "Sandbox Stories");
        assert_eq!(b.authors, vec!["Ada Lovelace", "Alan Turing"]);
        assert_eq!(b.series.as_deref(), Some("Foundations"));
        assert_eq!(b.media_type, MediaType::Ebook);
        assert_eq!(b.source_provider_id, "example-rest-catalog");
        assert_eq!(
            b.identifiers.get("isbn").map(String::as_str),
            Some("9781234567897")
        );
    }

    #[test]
    fn comma_separated_author_string_is_split() {
        let json = serde_json::json!({
            "results": [ { "id": "x", "title": "T", "authors": "A One, B Two" } ]
        });
        let books = map_response(&manifest(), &json);
        assert_eq!(books[0].authors, vec!["A One", "B Two"]);
    }

    // End-to-end via the SHIPPED example manifest file: load it from disk, then
    // run a fixture response through the same engine the app uses — proving the
    // whole plugin pipeline produces Books, entirely offline.
    #[test]
    fn shipped_example_plugin_produces_books_from_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("plugins")
            .join("example-rest-catalog.json");
        let loaded = super::super::load_one(&path).expect("example manifest loads + validates");

        // Sandbox holds for the manifest's own declared host.
        let req = resolve_request(
            &loaded.manifest,
            &serde_json::json!({ "base_url": "https://api.example-books.test", "api_key": "K" }),
        )
        .unwrap();
        assert!(req.url.starts_with("https://api.example-books.test"));

        let fixture = serde_json::json!({
            "results": [
                { "id": "1", "title": "The Pragmatic Plugin",
                  "authors": ["M. Author"], "identifiers": { "isbn_13": "9780000000001" } }
            ]
        });
        let books = map_response(&loaded.manifest, &fixture);
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].source_provider_id, "example-rest-catalog");
        assert_eq!(books[0].title, "The Pragmatic Plugin");
    }
}
