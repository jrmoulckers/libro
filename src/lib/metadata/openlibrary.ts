/**
 * Open Library metadata source (official public API, no key, **CORS-open**).
 *
 * Unlike the user-server connectors (OPDS/ABS), Open Library sends permissive
 * `Access-Control-Allow-Origin` headers, so these `fetch`es genuinely work from a
 * browser with no app-owned proxy — this is one of the two places (with Google
 * Books) where Libro can do real live network enrichment.
 *
 * Endpoint (batchable — many ISBNs in one request):
 *   `GET /api/books?bibkeys=ISBN:a,ISBN:b&format=json&jscmd=data`
 * Response is a map keyed by the bibkey; an unknown ISBN is simply absent (not a
 * 404). See <https://openlibrary.org/dev/docs/api/books>.
 *
 * The XML/JSON → {@link MetadataPatch} mapping is a **pure function**
 * ({@link parseOpenLibrary}) so tests feed fixture JSON with no network; the thin
 * {@link fetchOpenLibrary} shell just does the batched `fetch` + calls it.
 */

import { MetadataError, type MetadataPatch } from './types';

export const OPEN_LIBRARY_ID = 'openlibrary';
const BASE = 'https://openlibrary.org';

/** The bibkey the Open Library Books API uses for an ISBN. */
function bibkey(isbn: string): string {
  return `ISBN:${isbn}`;
}

/** Build the batched `/api/books` URL for a set of ISBNs. */
export function openLibraryBatchUrl(isbns: readonly string[]): string {
  const bibkeys = isbns.map(bibkey).join(',');
  return `${BASE}/api/books?bibkeys=${encodeURIComponent(bibkeys)}&format=json&jscmd=data`;
}

interface OlNamed {
  name?: unknown;
}
interface OlCover {
  large?: unknown;
  medium?: unknown;
  small?: unknown;
}
interface OlRecord {
  authors?: unknown;
  cover?: unknown;
  subjects?: unknown;
  series?: unknown;
  description?: unknown;
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function names(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((entry) => str((entry as OlNamed)?.name))
    .filter((n): n is string => n !== undefined);
}

/** Open Library returns descriptions as a plain string or `{ value }`. */
function description(value: unknown): string | undefined {
  if (typeof value === 'string') return str(value);
  if (value && typeof value === 'object') return str((value as { value?: unknown }).value);
  return undefined;
}

/**
 * Map one Open Library `/api/books` record (looked up by `isbn`) to a
 * {@link MetadataPatch}. Returns an empty patch when the ISBN is absent from the
 * response or carries none of the fields we enrich.
 */
export function parseOpenLibrary(json: unknown, isbn: string): MetadataPatch {
  if (!json || typeof json !== 'object') return {};
  const record = (json as Record<string, unknown>)[bibkey(isbn)] as OlRecord | undefined;
  if (!record || typeof record !== 'object') return {};

  const patch: MetadataPatch = {};

  const authors = names(record.authors);
  if (authors.length) patch.authors = authors;

  const cover = record.cover as OlCover | undefined;
  const coverUrl = str(cover?.large) ?? str(cover?.medium) ?? str(cover?.small);
  if (coverUrl) patch.coverUrl = coverUrl;

  const subjects = names(record.subjects);
  if (subjects.length) patch.subjects = subjects;

  // Series is usually absent from jscmd=data, but map it when present (string or
  // a one-element array).
  const series = Array.isArray(record.series) ? str(record.series[0]) : str(record.series);
  if (series) patch.series = series;

  // jscmd=data rarely includes a description; take it when it happens to be there.
  const desc = description(record.description);
  if (desc) patch.description = desc;

  return patch;
}

/** Map a whole batch response back to a per-ISBN {@link MetadataPatch}. */
export function parseOpenLibraryBatch(
  json: unknown,
  isbns: readonly string[],
): Map<string, MetadataPatch> {
  const out = new Map<string, MetadataPatch>();
  for (const isbn of isbns) out.set(isbn, parseOpenLibrary(json, isbn));
  return out;
}

/** Chunk `items` into runs of at most `size` (keeps batch URLs a sane length). */
export function chunk<T>(items: readonly T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += Math.max(1, size)) {
    out.push(items.slice(i, i + Math.max(1, size)));
  }
  return out;
}

const DEFAULT_CHUNK = 100;

/**
 * Fetch metadata for many ISBNs from Open Library, batched and chunked. The
 * `fetchImpl` is injected so tests supply a fake; the real caller passes the
 * global `fetch`. Throws {@link MetadataError} on a non-OK response so the caller
 * can isolate it — a *missing* ISBN just yields an empty patch in the map.
 */
export async function fetchOpenLibrary(
  isbns: readonly string[],
  fetchImpl: typeof fetch = fetch,
  chunkSize = DEFAULT_CHUNK,
): Promise<Map<string, MetadataPatch>> {
  const out = new Map<string, MetadataPatch>();
  for (const group of chunk(isbns, chunkSize)) {
    let json: unknown;
    try {
      const response = await fetchImpl(openLibraryBatchUrl(group));
      if (!response.ok) {
        throw new MetadataError(`HTTP ${response.status}`, OPEN_LIBRARY_ID);
      }
      json = await response.json();
    } catch (error) {
      if (error instanceof MetadataError) throw error;
      throw new MetadataError((error as Error).message, OPEN_LIBRARY_ID);
    }
    for (const [isbn, patch] of parseOpenLibraryBatch(json, group)) out.set(isbn, patch);
  }
  return out;
}
