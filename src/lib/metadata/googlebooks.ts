/**
 * Google Books metadata source (official public API, no key, **CORS-open**).
 *
 * Like Open Library, Google Books answers cross-origin browser requests, so this
 * `fetch` works with no app-owned proxy. It is used as the **fallback** after
 * Open Library — chiefly for the description OL's `jscmd=data` payload omits, but
 * also for a cover/authors/subjects when OL had none.
 *
 * Endpoint (one ISBN per request; no key needed for basic lookups):
 *   `GET /books/v1/volumes?q=isbn:{isbn}`
 * See <https://developers.google.com/books/docs/v1/using>.
 *
 * The JSON → {@link MetadataPatch} mapping is a **pure function**
 * ({@link parseGoogleBooks}); the thin {@link fetchGoogleBooks} shell does the
 * `fetch` + calls it.
 */

import { MetadataError, type MetadataPatch } from './types';

export const GOOGLE_BOOKS_ID = 'googlebooks';
const BASE = 'https://www.googleapis.com/books/v1/volumes';

/** Build the `/volumes` URL for an exact-ISBN lookup. */
export function googleBooksUrl(isbn: string): string {
  return `${BASE}?q=${encodeURIComponent(`isbn:${isbn}`)}&country=US&maxResults=1`;
}

interface GbImageLinks {
  thumbnail?: unknown;
  smallThumbnail?: unknown;
}
interface GbVolumeInfo {
  authors?: unknown;
  description?: unknown;
  categories?: unknown;
  imageLinks?: GbImageLinks;
}
interface GbVolume {
  volumeInfo?: GbVolumeInfo;
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function strings(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map(str).filter((s): s is string => s !== undefined);
}

/** Google often returns `http` thumbnails; upgrade them so they load on https. */
function https(url: string): string {
  return url.startsWith('http://') ? `https://${url.slice('http://'.length)}` : url;
}

/**
 * Map a Google Books volumes response to a {@link MetadataPatch}, using the first
 * volume. Returns an empty patch when there are no items or no enrichable fields.
 */
export function parseGoogleBooks(json: unknown): MetadataPatch {
  const items = (json as { items?: unknown })?.items;
  if (!Array.isArray(items) || items.length === 0) return {};
  const info = (items[0] as GbVolume)?.volumeInfo;
  if (!info || typeof info !== 'object') return {};

  const patch: MetadataPatch = {};

  const description = str(info.description);
  if (description) patch.description = description;

  const cover = str(info.imageLinks?.thumbnail) ?? str(info.imageLinks?.smallThumbnail);
  if (cover) patch.coverUrl = https(cover);

  const authors = strings(info.authors);
  if (authors.length) patch.authors = authors;

  const subjects = strings(info.categories);
  if (subjects.length) patch.subjects = subjects;

  // Google Books has no series concept, so `series` is intentionally never set.
  return patch;
}

/**
 * Fetch metadata for one ISBN from Google Books. `fetchImpl` is injected for
 * tests. Throws {@link MetadataError} on a non-OK response or Google's
 * `{ error }` envelope; a *no-results* response is an empty patch, not an error.
 */
export async function fetchGoogleBooks(
  isbn: string,
  fetchImpl: typeof fetch = fetch,
): Promise<MetadataPatch> {
  let json: unknown;
  try {
    const response = await fetchImpl(googleBooksUrl(isbn));
    if (!response.ok) {
      throw new MetadataError(`HTTP ${response.status}`, GOOGLE_BOOKS_ID);
    }
    json = await response.json();
  } catch (error) {
    if (error instanceof MetadataError) throw error;
    throw new MetadataError((error as Error).message, GOOGLE_BOOKS_ID);
  }
  const envelope = (json as { error?: { message?: unknown } })?.error;
  if (envelope) {
    throw new MetadataError(str(envelope.message) ?? 'Google Books API error', GOOGLE_BOOKS_ID);
  }
  return parseGoogleBooks(json);
}
