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
  /** Opaque text locator (EPUB CFI / foliate locator) used to resume reading. */
  locator?: string | null;
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

/**
 * A normalized audiobook chapter marker (mirrors the Rust `AudioChapter`).
 * Times are seconds from the start of the audiobook.
 */
export interface AudioChapter {
  id: number;
  start: number;
  end: number;
  title: string;
}

/**
 * A directly-playable audiobook stream + chapters (mirrors the Rust
 * `AudioPlayback`), returned by the `get_audiobook_stream` command. `stream_url`
 * is loadable straight into an `<audio>` element (auth token in the query
 * string, since a media element can't send an Authorization header).
 */
export interface AudioPlayback {
  stream_url: string;
  duration?: number | null;
  mime_type?: string | null;
  chapters: AudioChapter[];
}

/**
 * A minimal, frontend-facing view of an installed plugin (mirrors the Rust
 * `PluginInfo`), returned by the `list_plugins` command. Never carries the
 * user's secret config values — only what the loader validated.
 */
export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  author?: string | null;
  plugin_api_version: number;
  /** Raw capability bits (see the Rust `ProviderCapabilities`). */
  capabilities: number;
  /** Domains this plugin is sandboxed to (its only permitted network hosts). */
  allowed_domains: string[];
}
