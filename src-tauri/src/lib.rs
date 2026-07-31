//! Libro — the Tauri shell for the Libro pure-client media hub.
//!
//! All business logic lives in the [`libro_core`] crate (the normalized domain
//! model, the connector/plugin contract, and the config boundary). This crate
//! is a thin shell: it re-exposes `libro_core` to the React/TypeScript frontend
//! through the Tauri [`commands`] surface.

pub mod commands;

/// Build and run the Tauri application.
///
/// Kept separate from `main.rs` so the same entry point works for the desktop
/// binary and the mobile targets (which call this from generated glue).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(libro_core::sync::ReadingSyncState::new())
        .manage(libro_core::listening_sync::ListeningSyncState::new())
        .invoke_handler(tauri::generate_handler![
            commands::list_all_books,
            commands::list_books_by_provider,
            commands::search_metadata,
            commands::lookup_metadata_by_isbn,
            commands::get_local_cover,
            commands::get_book_file,
            commands::save_reading_progress,
            commands::get_reading_progress,
            commands::get_audiobook_stream,
            commands::save_listening_progress,
            commands::get_listening_progress,
            commands::list_plugins
        ])
        .run(tauri::generate_context!())
        .expect("error while running Libro");
}
