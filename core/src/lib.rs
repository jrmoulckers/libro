//! Libro core — the pure-client business logic for Libro.
//!
//! This crate is intentionally **UI- and Tauri-free**. It holds the normalized
//! domain model, the connector/plugin contract, and the local configuration
//! boundary. The Tauri shell (`libro`) depends on this crate and exposes it to
//! the React frontend through a thin command surface.
//!
//! Module map:
//! * [`models`] — the normalized, provider-agnostic domain model.
//! * [`providers`] — the connector/plugin contract ([`providers::Provider`]).
//! * [`metadata`] — the metadata-enrichment contract
//!   ([`metadata::MetadataProvider`]); distinct from library `Provider`s.
//! * [`config`] — local, encrypted-at-rest configuration (boundary only).
//! * [`sync`] — reading-progress sync to tracking services (e.g. Hardcover).
//! * [`plugins`] — the declarative plugin SDK for third-party connectors.
//!
//! Keeping this logic in its own crate means the mapping/aggregation code can be
//! unit-tested with a plain static test binary, independent of the WebView
//! runtime that the desktop/mobile shell links against.

pub mod config;
pub mod metadata;
pub mod models;
pub mod plugins;
pub mod providers;
pub mod sync;
