/**
 * Shared TypeScript types that mirror the normalized Rust domain model
 * (see `src-tauri/src/models`). Keep these in sync with the Rust side; a
 * future phase may codegen them (e.g. via `ts-rs` or `specta`).
 */

export type MediaType = "Ebook" | "Audiobook" | "Podcast";

export interface Progress {
  /** 0.0 – 1.0 fractional completion. */
  fraction: number;
  /** Last playback/reading position in seconds (audio) or locator (text). */
  position_seconds?: number | null;
  /** Whether the item is marked finished. */
  finished: boolean;
}

export interface Book {
  id: string;
  title: string;
  authors: string[];
  series?: string | null;
  cover_url?: string | null;
  /** Map of identifier scheme -> value, e.g. { isbn: "...", asin: "..." }. */
  identifiers: Record<string, string>;
  media_type: MediaType;
  source_provider_id: string;
  progress?: Progress | null;
}
