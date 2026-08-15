/**
 * Audiobookshelf (ABS) connector — Libro's first audiobook source.
 *
 * Audiobookshelf is a self-hosted audiobook/podcast/ebook server the user runs
 * themselves. Libro talks only to the user's *own* server over its official REST
 * API, authenticating with a per-user **API token** sent as a Bearer header.
 *
 * Endpoints used this phase:
 *  - `GET {baseUrl}/api/libraries`            — enumerate libraries.
 *  - `GET {baseUrl}/api/libraries/{id}/items` — list a library's items.
 *  - `GET {baseUrl}/api/items/{id}/cover`     — per-item cover (referenced in URLs).
 *
 * ## Browser constraint — CORS (same reality as OPDS)
 * Browser `fetch` to the user's ABS server only succeeds if that server returns a
 * permissive `Access-Control-Allow-Origin` (or is same-origin). Libro is
 * pure-client with **no app-owned proxy** (studio rule: no server tier). A failing
 * fetch throws a typed {@link AbsError} so {@link ../registry.aggregateLibrary}'s
 * `Promise.allSettled` isolates it — an unreachable ABS server never crashes the
 * aggregate.
 *
 * ## Cover-image caveat
 * The cover endpoint is protected. A browser image element cannot send an
 * `Authorization` header, so — mirroring the blueprint — the token is appended as a
 * `?token=` query param when available, making the URL directly usable as an image
 * source. When no token is configured the plain URL is used (public covers only).
 *
 * ## Pure/testable split
 * {@link mapAbsLibraryItems} / {@link mapAbsItem} are pure `JSON -> Book` functions
 * with no network, unit-tested against sample ABS payloads. `listBooks()` is just
 * the fetch(es) + the pure mapper.
 *
 * ## Phase 8+ TODOs (documented, not built here)
 *  - `progress-sync` is advertised now (ABS is our listening-progress source), but
 *    only catalog fetch is implemented. Pull (`GET /api/me` mediaProgress) and push
 *    land in P8; merge onto `Book.progress` by `abs:item_id`.
 *  - Token via `POST /api/login` (we take a user-supplied API token for now).
 *  - Richer media-type detection (podcast/ebook vs audiobook) from library type /
 *    `numTracks` / `ebookFormat`; this phase maps everything as `audiobook`.
 */

import type { Book } from '../models';
import type { Provider, ProviderCapability } from './types';

/** Configuration for a single Audiobookshelf connector instance. */
export interface AbsConfig {
  /** Stable provider id, used as {@link Book.sourceProviderId}. */
  id: string;
  /** Human-friendly name for the UI. */
  displayName: string;
  /** ABS server root, e.g. `https://abs.example.com`. */
  baseUrl: string;
  /** Per-user API token, sent as `Authorization: Bearer <apiToken>`. */
  apiToken: string;
  /** Restrict to one library; when omitted, every library is aggregated. */
  libraryId?: string;
}

/** Typed error thrown by ABS fetch/parse failures. */
export class AbsError extends Error {
  constructor(
    message: string,
    override readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'AbsError';
  }
}

/**
 * Map an ABS `/api/libraries/{id}/items` payload (or a raw items array) into
 * normalized {@link Book}s. Pure: no network.
 *
 * Accepts either the full response object (`{ results: [...] }`) or a bare array
 * of items, and tolerates unknown/partial shapes (unrecognized entries are
 * skipped rather than throwing).
 */
export function mapAbsLibraryItems(
  items: unknown,
  baseUrl: string,
  providerId: string,
  apiToken?: string,
): Book[] {
  const list = asArray(items) ?? asArray(asRecord(items)?.results) ?? [];
  const books: Book[] = [];
  for (const raw of list) {
    const book = mapAbsItem(raw, baseUrl, providerId, apiToken);
    if (book) {
      books.push(book);
    }
  }
  return books;
}

/**
 * Map a single ABS library item into a {@link Book}, or `null` when it lacks a
 * usable id or title. Pure: no network.
 */
export function mapAbsItem(
  item: unknown,
  baseUrl: string,
  providerId: string,
  apiToken?: string,
): Book | null {
  const record = asRecord(item);
  if (!record) {
    return null;
  }

  const itemId = asString(record.id);
  const media = asRecord(record.media);
  const metadata = asRecord(media?.metadata) ?? {};
  const title = asString(metadata.title);
  if (!itemId || !title) {
    return null;
  }

  const base = normalizeBaseUrl(baseUrl);

  const identifiers: Record<string, string> = { 'abs:item_id': itemId };
  const isbn = asString(metadata.isbn);
  if (isbn) {
    identifiers.isbn = isbn;
  }
  const asin = asString(metadata.asin);
  if (asin) {
    identifiers.asin = asin;
  }

  const book: Book = {
    id: `abs-${itemId}`,
    title,
    authors: readAuthors(metadata),
    mediaType: 'audiobook',
    sourceProviderId: providerId,
    identifiers,
  };

  const series = readSeries(metadata);
  if (series) {
    book.series = series;
  }

  const description = asString(metadata.description);
  if (description) {
    book.description = description;
  }

  // Cover is only referenced when the item actually has one on the server.
  if (asString(media?.coverPath)) {
    const query = apiToken ? `?token=${encodeURIComponent(apiToken)}` : '';
    book.coverUrl = `${base}/api/items/${encodeURIComponent(itemId)}/cover${query}`;
  }

  return book;
}

/**
 * Create an ABS provider from config. Advertises `catalog` (it enumerates a
 * library) and `progress-sync` (ABS is Libro's listening-progress source; the
 * actual sync lands in a later phase — see the file header).
 */
export function createAbsProvider(config: AbsConfig): Provider {
  const capabilities: ReadonlySet<ProviderCapability> = new Set(['catalog', 'progress-sync']);
  const base = normalizeBaseUrl(config.baseUrl);

  async function fetchJson(url: string): Promise<unknown> {
    let response: Response;
    try {
      response = await fetch(url, {
        headers: { Accept: 'application/json', Authorization: `Bearer ${config.apiToken}` },
      });
    } catch (cause) {
      // Network/CORS failures land here — see the CORS note at the top of file.
      throw new AbsError(`Audiobookshelf request to ${url} failed`, cause);
    }
    if (!response.ok) {
      throw new AbsError(`Audiobookshelf request to ${url} returned ${response.status}`);
    }
    try {
      return await response.json();
    } catch (cause) {
      throw new AbsError(`Audiobookshelf returned malformed JSON from ${url}`, cause);
    }
  }

  return {
    id: config.id,
    displayName: config.displayName,
    capabilities,
    async listBooks(): Promise<Book[]> {
      const libraryIds = config.libraryId
        ? [config.libraryId]
        : readLibraryIds(await fetchJson(`${base}/api/libraries`));

      const books: Book[] = [];
      for (const libraryId of libraryIds) {
        const payload = await fetchJson(
          `${base}/api/libraries/${encodeURIComponent(libraryId)}/items`,
        );
        books.push(...mapAbsLibraryItems(payload, base, config.id, config.apiToken));
      }
      return books;
    },
  };
}

/** Extract library ids from a `/api/libraries` response. */
function readLibraryIds(payload: unknown): string[] {
  const libraries = asArray(asRecord(payload)?.libraries) ?? [];
  return libraries
    .map((lib) => asString(asRecord(lib)?.id))
    .filter((id): id is string => Boolean(id));
}

/** Authors from full (`authors: [{name}]`) or minified (`authorName` CSV) metadata. */
function readAuthors(metadata: Record<string, unknown>): string[] {
  const structured = asArray(metadata.authors);
  if (structured) {
    return structured
      .map((a) => asString(asRecord(a)?.name) ?? asString(a))
      .filter((name): name is string => Boolean(name));
  }
  const authorName = asString(metadata.authorName);
  if (authorName) {
    return authorName
      .split(',')
      .map((a) => a.trim())
      .filter(Boolean);
  }
  return [];
}

/** Series from full (`series: [{name}]`/string) or minified (`seriesName`) metadata. */
function readSeries(metadata: Record<string, unknown>): string | undefined {
  const structured = asArray(metadata.series);
  if (structured && structured.length > 0) {
    const first = structured[0];
    return asString(asRecord(first)?.name) ?? asString(first);
  }
  return asString(metadata.series) ?? asString(metadata.seriesName);
}

function normalizeBaseUrl(url: string): string {
  return url.replace(/\/+$/, '');
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function asArray(value: unknown): unknown[] | undefined {
  return Array.isArray(value) ? value : undefined;
}

/** Non-empty trimmed string, or `undefined`. */
function asString(value: unknown): string | undefined {
  if (typeof value !== 'string') {
    return undefined;
  }
  const trimmed = value.trim();
  return trimmed ? trimmed : undefined;
}
