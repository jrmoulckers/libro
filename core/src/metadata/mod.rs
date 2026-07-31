//! Metadata enrichment — a distinct abstraction from library [`Provider`]s.
//!
//! A [`MetadataProvider`] answers *"tell me about this book"* from an official
//! public bibliographic API (Open Library, Google Books). It is **not** a source
//! of the user's owned library, so it deliberately does **not** implement the
//! [`crate::providers::Provider`] contract and takes no part in the
//! `list_all_books` aggregation fan-out. Modelling it as its own trait keeps the
//! two concerns cleanly separated:
//!
//! * `providers::Provider`  → "what does the user own / can act on?" (per-account)
//! * `metadata::MetadataProvider` → "what is the canonical data for a title?"
//!
//! The [`enrich`] helper is the backbone catalog connectors reuse: it fills in
//! fields a source provider left blank (cover, description, series, identifiers)
//! from resolved [`BookMetadata`], **without** overwriting anything already set.
//!
//! [`Provider`]: crate::providers::Provider

pub mod catalog;
pub mod googlebooks;
pub mod openlibrary;

use async_trait::async_trait;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::models::Book;

pub use catalog::{enrich_catalog, EnrichOptions};
pub use googlebooks::GoogleBooksProvider;
pub use openlibrary::OpenLibraryProvider;

/// `User-Agent` sent with every metadata request. Open Library asks callers to
/// identify themselves; the others tolerate it.
pub(crate) const USER_AGENT: &str =
    concat!("Libro/", env!("CARGO_PKG_VERSION"), " (metadata enrichment)");

/// Error surfaced by a [`MetadataProvider`].
///
/// Note: a *missing* record (HTTP 404 or an empty result set) is **not** an
/// error — it is `Ok(None)` / an empty `Vec`. Errors are reserved for transport
/// failures, rate limiting, and unexpected/non-JSON responses.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("network error: {0}")]
    Network(String),
    #[error("API error: {0}")]
    Api(String),
    #[error("unexpected error: {0}")]
    Other(String),
}

/// Result alias for metadata operations.
pub type MetadataResult<T> = Result<T, MetadataError>;

/// Normalized bibliographic metadata for a single edition/work.
///
/// This is richer than [`Book`] (which is a *catalog item* the user has), and is
/// intentionally provider-agnostic: Open Library and Google Books both map onto
/// it so the rest of the app deals with one shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BookMetadata {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    /// Identifier scheme -> value, e.g. `isbn13`, `isbn10`, `olid`,
    /// `google_volume_id`. A `BTreeMap` keeps serialization deterministic.
    #[serde(default)]
    pub identifiers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The [`MetadataProvider::id`] that produced this record.
    #[serde(default)]
    pub source: String,
}

/// The metadata-enrichment contract.
///
/// Implement this to add a bibliographic source. All methods are `async`
/// (network I/O) and object-safe via [`async_trait`], so providers can be stored
/// as `Box<dyn MetadataProvider>` in a [`MetadataRegistry`].
#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Stable, machine-readable id (e.g. `"openlibrary"`).
    fn id(&self) -> &str;

    /// Resolve metadata for an exact ISBN (10 or 13). `Ok(None)` when not found.
    async fn by_isbn(&self, isbn: &str) -> MetadataResult<Option<BookMetadata>>;

    /// Free-text search (title/author/keywords), capped at `limit` results.
    async fn search(&self, query: &str, limit: usize) -> MetadataResult<Vec<BookMetadata>>;

    /// Resolve by a named identifier scheme. The default handles ISBN schemes and
    /// returns `Ok(None)` for anything a provider doesn't specifically support.
    async fn by_identifier(&self, kind: &str, value: &str) -> MetadataResult<Option<BookMetadata>> {
        match kind.to_ascii_lowercase().as_str() {
            "isbn" | "isbn10" | "isbn13" => self.by_isbn(value).await,
            _ => Ok(None),
        }
    }
}

/// A small, ordered registry of enabled metadata providers.
///
/// Providers are consulted in priority order: [`by_isbn`](Self::by_isbn) returns
/// the first hit, and [`search`](Self::search) returns the first non-empty result
/// set. Open Library (no auth) is always available; Google Books is added with an
/// optional API key from [`AppConfig`].
pub struct MetadataRegistry {
    providers: Vec<Box<dyn MetadataProvider>>,
}

impl MetadataRegistry {
    /// Build a registry from application config.
    pub fn from_config(config: &AppConfig) -> Self {
        let providers: Vec<Box<dyn MetadataProvider>> = vec![
            Box::new(OpenLibraryProvider::new()),
            Box::new(GoogleBooksProvider::new(
                config.metadata.google_books_api_key.clone(),
            )),
        ];
        Self { providers }
    }

    /// Construct from an explicit provider list (used in tests).
    pub fn new(providers: Vec<Box<dyn MetadataProvider>>) -> Self {
        Self { providers }
    }

    /// The ids of the registered providers, in priority order.
    pub fn provider_ids(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.id()).collect()
    }

    /// Resolve an ISBN across providers, returning the first hit.
    ///
    /// A provider error is logged and treated as a miss so one flaky source can't
    /// block a lookup another source could satisfy.
    pub async fn by_isbn(&self, isbn: &str) -> MetadataResult<Option<BookMetadata>> {
        for p in &self.providers {
            match p.by_isbn(isbn).await {
                Ok(Some(meta)) => return Ok(Some(meta)),
                Ok(None) => continue,
                Err(e) => eprintln!("libro: metadata provider '{}' by_isbn error: {e}", p.id()),
            }
        }
        Ok(None)
    }

    /// Search across providers, returning the first non-empty result set.
    pub async fn search(&self, query: &str, limit: usize) -> MetadataResult<Vec<BookMetadata>> {
        for p in &self.providers {
            match p.search(query, limit).await {
                Ok(results) if !results.is_empty() => return Ok(results),
                Ok(_) => continue,
                Err(e) => eprintln!("libro: metadata provider '{}' search error: {e}", p.id()),
            }
        }
        Ok(Vec::new())
    }
}

/// Fill **missing** fields on `book` from resolved `meta`.
///
/// Existing values are never overwritten — enrichment only augments. This is the
/// primitive catalog connectors (ABS/LazyLibrarian) reuse after resolving a book
/// against a [`MetadataProvider`].
pub fn enrich(book: &mut Book, meta: &BookMetadata) {
    if book.authors.is_empty() && !meta.authors.is_empty() {
        book.authors = meta.authors.clone();
    }
    if book.cover_url.is_none() {
        if let Some(cover) = &meta.cover_url {
            book.cover_url = Some(cover.clone());
        }
    }
    if book.description.is_none() {
        if let Some(desc) = &meta.description {
            book.description = Some(desc.clone());
        }
    }
    if book.series.is_none() {
        if let Some(series) = &meta.series {
            book.series = Some(series.clone());
        }
    }
    // Add any identifier the book doesn't already carry; never replace.
    for (scheme, value) in &meta.identifiers {
        book.identifiers
            .entry(scheme.clone())
            .or_insert_with(|| value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Book, MediaType};

    fn sample_meta() -> BookMetadata {
        BookMetadata {
            title: "Effective Java".into(),
            authors: vec!["Joshua Bloch".into()],
            description: Some("A best-practices guide.".into()),
            cover_url: Some("https://covers.example/ej-L.jpg".into()),
            series: Some("The Java Series".into()),
            identifiers: BTreeMap::from([
                ("isbn13".to_string(), "9780134685991".to_string()),
                ("olid".to_string(), "OL31838212M".to_string()),
            ]),
            source: "openlibrary".into(),
            ..Default::default()
        }
    }

    #[test]
    fn enrich_fills_only_missing_fields() {
        let mut book = Book::new("x", "Effective Java", MediaType::Ebook, "audiobookshelf");
        // Pre-existing cover + one identifier should be preserved.
        book.cover_url = Some("https://existing.example/cover.jpg".into());
        book.identifiers
            .insert("isbn13".into(), "0000000000000".into());

        enrich(&mut book, &sample_meta());

        // Missing fields filled.
        assert_eq!(book.authors, vec!["Joshua Bloch"]);
        assert_eq!(book.description.as_deref(), Some("A best-practices guide."));
        assert_eq!(book.series.as_deref(), Some("The Java Series"));
        assert_eq!(book.identifiers.get("olid").map(String::as_str), Some("OL31838212M"));

        // Existing values NOT overwritten.
        assert_eq!(
            book.cover_url.as_deref(),
            Some("https://existing.example/cover.jpg")
        );
        assert_eq!(
            book.identifiers.get("isbn13").map(String::as_str),
            Some("0000000000000")
        );
    }

    #[test]
    fn enrich_is_noop_when_nothing_missing() {
        let mut book = Book::new("x", "Title", MediaType::Ebook, "src");
        book.authors = vec!["Real Author".into()];
        book.cover_url = Some("c".into());
        book.description = Some("d".into());
        book.series = Some("s".into());
        let before = book.clone();

        enrich(&mut book, &sample_meta());
        // Only identifiers may have grown; core fields unchanged.
        assert_eq!(book.authors, before.authors);
        assert_eq!(book.cover_url, before.cover_url);
        assert_eq!(book.description, before.description);
        assert_eq!(book.series, before.series);
    }
}
