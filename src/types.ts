/**
 * Shared TypeScript types that mirror the normalized Rust domain model
 * (see `core/src/models`). Keep these in sync with the Rust side; a
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
  description?: string | null;
  /** Map of identifier scheme -> value, e.g. { isbn: "...", asin: "..." }. */
  identifiers: Record<string, string>;
  media_type: MediaType;
  source_provider_id: string;
  progress?: Progress | null;
}

/**
 * Normalized bibliographic metadata (mirrors the Rust `metadata::BookMetadata`).
 * Produced by the metadata-enrichment layer (Open Library / Google Books), which
 * is distinct from the library `Provider`s that list a user's owned catalog.
 */
export interface BookMetadata {
  title: string;
  subtitle?: string | null;
  authors: string[];
  description?: string | null;
  cover_url?: string | null;
  series?: string | null;
  identifiers: Record<string, string>;
  publish_date?: string | null;
  page_count?: number | null;
  publisher?: string | null;
  language?: string | null;
  /** The metadata provider id that produced this record. */
  source: string;
}

/**
 * Per-provider aggregation result (mirrors the Rust `ProviderBooks`).
 * Lets the UI show a per-provider error state instead of failing the whole
 * catalog when one connector is misconfigured or offline.
 */
export interface ProviderBooks {
  provider_id: string;
  display_name: string;
  /** Raw capability bits (see the Rust `ProviderCapabilities`). */
  capabilities: number;
  books: Book[];
  /** `null` on success; a human-readable message on failure. */
  error?: string | null;
}
