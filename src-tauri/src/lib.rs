//! Libro core library.
//!
//! Libro is a cross-platform, **pure-client** media hub for books and
//! audiobooks. All business logic lives here in Rust (the Tauri core); the
//! React/TypeScript frontend is a thin UI over the [`commands`] surface.
//!
//! Module map:
//! * [`models`] — the normalized, provider-agnostic domain model.
//! * [`providers`] — the connector/plugin contract ([`providers::Provider`]).
//! * [`config`] — local, encrypted-at-rest configuration (boundary only).
//! * [`commands`] — Tauri commands invoked from the frontend.

pub mod commands;
pub mod config;
pub mod models;
pub mod providers;

/// Build and run the Tauri application.
///
/// Kept separate from `main.rs` so the same entry point works for the desktop
/// binary and the mobile targets (which call this from generated glue).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::list_all_books])
        .run(tauri::generate_context!())
        .expect("error while running Libro");
}
