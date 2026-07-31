//! Libby / OverDrive placeholder connector — **deep-link-only**.
//!
//! Libby (OverDrive) is a walled garden: there is **no official public API** for
//! third-party clients to place holds or borrow titles. Libro therefore does
//! **not** integrate it — this connector exists to encode that legal boundary in
//! code. It holds the user's library identifier and produces a **deep link into
//! the official Libby app**; it must never call any OverDrive endpoint, scrape,
//! or reverse-engineer a private ("Thunder") API.
//!
//! Capability: [`ProviderCapabilities::DEEP_LINK_ONLY`] (nothing else).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::Book;
use crate::providers::{Provider, ProviderCapabilities, ProviderResult};

/// Settings for the Libby deep-link placeholder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibbyConfig {
    /// The OverDrive/Libby library key (slug), e.g. `"lapl"` for
    /// `https://libbyapp.com/library/lapl`. Optional.
    #[serde(default)]
    pub library_key: String,
    /// Optional user-facing card/library label for display.
    #[serde(default)]
    pub label: Option<String>,
}

/// The Libby deep-link placeholder connector.
pub struct LibbyProvider {
    config: LibbyConfig,
}

impl LibbyProvider {
    pub const ID: &'static str = "libby";

    pub fn new(config: LibbyConfig) -> Self {
        Self { config }
    }

    /// Build a deep link into the official Libby web/app experience.
    ///
    /// With a `search` term this links to the library's search; otherwise it
    /// opens the configured library home. This is the *only* outbound behavior
    /// this connector has — it performs no API calls.
    pub fn deep_link(&self, search: Option<&str>) -> String {
        let key = if self.config.library_key.is_empty() {
            // No configured library: fall back to Libby's library picker.
            return "https://libbyapp.com/shelf/libraries".to_string();
        } else {
            self.config.library_key.as_str()
        };
        match search {
            Some(q) if !q.is_empty() => {
                let encoded = urlencode(q);
                format!("https://libbyapp.com/search/{key}/search/query-{encoded}/page-1")
            }
            _ => format!("https://libbyapp.com/library/{key}"),
        }
    }
}

/// Minimal percent-encoding for the query segment (no external dep needed).
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[async_trait]
impl Provider for LibbyProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "Libby (OverDrive)"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Walled garden: surfaced only as an "open in the official app" link.
        ProviderCapabilities::DEEP_LINK_ONLY
    }

    async fn authenticate(&mut self, config: &serde_json::Value) -> ProviderResult<()> {
        // No network auth: a deep-link-only provider never contacts OverDrive.
        if !config.is_null() {
            if let Ok(cfg) = serde_json::from_value::<LibbyConfig>(config.clone()) {
                self.config = cfg;
            }
        }
        Ok(())
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        // Deep-link-only providers contribute no catalog items; the UI surfaces
        // them via `deep_link()` instead. Return empty rather than an error so
        // aggregation stays quiet.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_link_to_library_home_when_no_search() {
        let p = LibbyProvider::new(LibbyConfig {
            library_key: "lapl".into(),
            label: None,
        });
        assert_eq!(p.deep_link(None), "https://libbyapp.com/library/lapl");
    }

    #[test]
    fn deep_link_encodes_search_query() {
        let p = LibbyProvider::new(LibbyConfig {
            library_key: "lapl".into(),
            label: None,
        });
        assert_eq!(
            p.deep_link(Some("the hobbit")),
            "https://libbyapp.com/search/lapl/search/query-the%20hobbit/page-1"
        );
    }

    #[test]
    fn deep_link_falls_back_to_picker_without_library_key() {
        let p = LibbyProvider::new(LibbyConfig::default());
        assert_eq!(p.deep_link(None), "https://libbyapp.com/shelf/libraries");
    }
}
