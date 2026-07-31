//! Normalized domain model shared across every provider/connector.
//!
//! Providers translate their own API responses into these types so the rest of
//! the application only ever deals with one canonical shape. Keep this model
//! provider-agnostic: nothing here should assume a particular backend.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The kind of media a [`Book`] represents.
///
/// `Book` is the historical name for "a catalog item"; it also covers
/// audiobooks and podcasts to keep the aggregation layer uniform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    Ebook,
    Audiobook,
    Podcast,
}

/// Reading/listening progress for an item.
///
/// This is intentionally small for the skeleton; a later phase will expand it
/// with per-device positions and a conflict-resolution strategy for the
/// Signal-style device-to-device sync model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Fractional completion in the range `0.0..=1.0`.
    pub fraction: f32,
    /// Last position in seconds (audio) or an opaque locator offset (text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_seconds: Option<f64>,
    /// Whether the user has marked the item finished.
    pub finished: bool,
}

/// A single normalized catalog item.
///
/// Every connector maps its native representation onto this struct. The
/// `identifiers` map holds cross-provider keys (ISBN, ASIN, etc.) that later
/// phases use to de-duplicate the same title coming from multiple providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    /// Stable id **within the source provider** (not globally unique on its own;
    /// combine with `source_provider_id`).
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Identifier scheme -> value, e.g. `{"isbn": "…", "asin": "…"}`.
    ///
    /// A `BTreeMap` keeps serialization stable/deterministic.
    #[serde(default)]
    pub identifiers: BTreeMap<String, String>,
    pub media_type: MediaType,
    /// The [`crate::providers::Provider::id`] of the connector this item came from.
    pub source_provider_id: String,
    /// Optional progress; `None` when unknown or not yet synced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
}

impl Book {
    /// Convenience constructor for the minimal required fields.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        media_type: MediaType,
        source_provider_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            authors: Vec::new(),
            series: None,
            cover_url: None,
            identifiers: BTreeMap::new(),
            media_type,
            source_provider_id: source_provider_id.into(),
            progress: None,
        }
    }
}
