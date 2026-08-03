/**
 * Generic OPDS (Open Publication Distribution System) catalog connector.
 *
 * OPDS is an Atom-based open standard, so one connector reaches many servers:
 * Calibre-Web, Calibre's content server, Kavita, Komga, and public catalogs such
 * as Standard Ebooks and Project Gutenberg.
 *
 * ## Browser constraint — CORS (read this before wiring a real catalog)
 * Unlike the native/Tauri blueprint, browser `fetch` to a user's OPDS server is
 * subject to the same-origin policy: it only succeeds if that server returns a
 * permissive `Access-Control-Allow-Origin` header (or is served same-origin as
 * the app). Libro is pure-client with **no app-owned proxy** (studio rule: no
 * server tier, ever), so catalogs behind a CORS-closed server cannot be read from
 * the browser. That is a deployment property of the user's server, not a bug here.
 *
 * `listBooks()` therefore throws a typed {@link OpdsError} on any fetch/parse
 * failure so {@link ../registry.aggregateLibrary}'s `Promise.allSettled` isolates
 * it — one unreachable OPDS server never crashes the aggregate.
 *
 * ## Pure/testable split
 * {@link parseOpdsFeed} is a pure `xml -> Book[]` function with no network, so it
 * can be unit-tested against sample XML. `listBooks()` is just `fetch` + parse.
 *
 * ## Phase 3+ TODOs (documented, not built here)
 *  - Pagination: only the first page is read; follow `rel="next"` later.
 *  - Navigation feeds: crawl `rel="subsection"` nav entries down to acquisition
 *    feeds. For now, non-acquisition entries are simply skipped.
 *  - `download` capability: fetch the acquisition URL bytes (needs the same CORS
 *    caveat); the URL is already carried in `identifiers['opds:acquisition_url']`.
 */

import type { Book } from '../models';
import type { Provider, ProviderCapability } from './types';

const REL_ACQUISITION_PREFIX = 'http://opds-spec.org/acquisition';
const REL_IMAGE = 'http://opds-spec.org/image';
const REL_THUMBNAIL = 'http://opds-spec.org/image/thumbnail';

/** Optional HTTP Basic credentials for a protected catalog. */
export interface OpdsAuth {
  username: string;
  password: string;
}

/** Configuration for a single OPDS connector instance. */
export interface OpdsConfig {
  /** Stable provider id, used as {@link Book.sourceProviderId}. */
  id: string;
  /** Human-friendly name for the UI. */
  displayName: string;
  /** Root/acquisition feed URL to fetch. */
  catalogUrl: string;
  /** HTTP Basic credentials; when omitted, no auth header is sent. */
  auth?: OpdsAuth;
}

/** Typed error thrown by OPDS fetch/parse failures. */
export class OpdsError extends Error {
  constructor(
    message: string,
    readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'OpdsError';
  }
}

/** Build an `Authorization: Basic …` header value from credentials. */
export function basicAuthHeader(auth: OpdsAuth): string {
  return `Basic ${btoa(`${auth.username}:${auth.password}`)}`;
}

/**
 * Parse an OPDS Atom feed document into normalized {@link Book}s. Pure: no network.
 *
 * Each `<entry>` with a title and at least one acquisition link becomes a book;
 * relative links are resolved to absolute URLs against `catalogUrl`. Entries
 * without a title or acquisition link (e.g. navigation entries) are skipped.
 *
 * @throws {OpdsError} when the document is not well-formed XML.
 */
export function parseOpdsFeed(xml: string, catalogUrl: string, providerId: string): Book[] {
  const doc = new DOMParser().parseFromString(xml, 'application/xml');
  if (doc.getElementsByTagName('parsererror').length > 0) {
    throw new OpdsError('OPDS feed is not well-formed XML');
  }

  const books: Book[] = [];
  for (const entry of Array.from(doc.getElementsByTagNameNS('*', 'entry'))) {
    const book = mapEntry(entry, catalogUrl, providerId);
    if (book) {
      books.push(book);
    }
  }
  return books;
}

interface Acquisition {
  href: string;
  rel: string;
  type: string | null;
}

function mapEntry(entry: Element, base: string, providerId: string): Book | null {
  const title = childText(entry, 'title');
  if (!title) {
    return null;
  }

  const acquisitions: Acquisition[] = [];
  let image: string | undefined;
  let thumbnail: string | undefined;

  for (const link of directChildren(entry, 'link')) {
    const href = link.getAttribute('href');
    if (!href) {
      continue;
    }
    const rel = link.getAttribute('rel') ?? '';
    const abs = resolveUrl(base, href);

    if (rel.startsWith(REL_ACQUISITION_PREFIX)) {
      acquisitions.push({ href: abs, rel, type: link.getAttribute('type') });
    } else if (rel === REL_IMAGE) {
      image ??= abs;
    } else if (rel === REL_THUMBNAIL) {
      thumbnail ??= abs;
    }
  }

  if (acquisitions.length === 0) {
    return null;
  }

  // Prefer an open-access link, then an EPUB, then whatever came first.
  const primary =
    acquisitions.find((a) => a.rel.includes('open-access')) ??
    acquisitions.find((a) => a.type?.includes('epub')) ??
    acquisitions[0];

  const authors = directChildren(entry, 'author')
    .map((a) => childText(a, 'name'))
    .filter((name): name is string => Boolean(name));

  const identifiers: Record<string, string> = {
    'opds:acquisition_url': primary.href,
  };
  if (primary.type) {
    identifiers['opds:acquisition_type'] = primary.type;
  }

  const book: Book = {
    // Stable id derived from the (absolute) acquisition URL; falls back to the
    // entry <id> only if an acquisition URL is somehow absent (unreachable here,
    // since entries without one are skipped above).
    id: primary.href ? `opds-${hashString(primary.href)}` : childText(entry, 'id') || primary.href,
    title,
    authors,
    mediaType: 'ebook',
    sourceProviderId: providerId,
    identifiers,
  };

  const description = childText(entry, 'summary') || childText(entry, 'content');
  if (description) {
    book.description = description;
  }

  const cover = image ?? thumbnail;
  if (cover) {
    book.coverUrl = cover;
  }

  const series = extractSeries(entry);
  if (series) {
    book.series = series;
  }

  return book;
}

function extractSeries(entry: Element): string | undefined {
  for (const node of Array.from(entry.children)) {
    if (node.localName === 'series') {
      const text = node.textContent?.trim();
      if (text) {
        return text;
      }
    }
    if (node.localName === 'category') {
      const isSeries = node.getAttribute('scheme')?.toLowerCase().includes('series');
      if (isSeries) {
        const value = (node.getAttribute('label') ?? node.getAttribute('term'))?.trim();
        if (value) {
          return value;
        }
      }
    }
  }
  return undefined;
}

/** Direct child elements of `parent` with the given local name. */
function directChildren(parent: Element, localName: string): Element[] {
  return Array.from(parent.children).filter((child) => child.localName === localName);
}

/** Trimmed text of the first direct child element with the given local name. */
function childText(parent: Element, localName: string): string {
  const child = Array.from(parent.children).find((c) => c.localName === localName);
  return child?.textContent?.trim() ?? '';
}

/** Resolve `href` (possibly relative) against the feed `base` URL. */
function resolveUrl(base: string, href: string): string {
  try {
    return new URL(href, base).toString();
  } catch {
    return href;
  }
}

/** Small deterministic string hash (djb2), base36 — used for stable book ids. */
function hashString(input: string): string {
  let hash = 5381;
  for (let i = 0; i < input.length; i++) {
    hash = ((hash << 5) + hash + input.charCodeAt(i)) >>> 0;
  }
  return hash.toString(36);
}

/**
 * Create an OPDS provider from config. Advertises `catalog` (it enumerates a
 * library) and `download` (each entry's acquisition link is a downloadable file).
 */
export function createOpdsProvider(config: OpdsConfig): Provider {
  const capabilities: ReadonlySet<ProviderCapability> = new Set(['catalog', 'download']);

  return {
    id: config.id,
    displayName: config.displayName,
    capabilities,
    async listBooks(): Promise<Book[]> {
      let response: Response;
      try {
        response = await fetch(config.catalogUrl, {
          headers: {
            Accept: 'application/atom+xml, application/xml, text/xml',
            ...(config.auth ? { Authorization: basicAuthHeader(config.auth) } : {}),
          },
        });
      } catch (cause) {
        // Network/CORS failures land here — see the CORS note at the top of file.
        throw new OpdsError(`OPDS request to ${config.catalogUrl} failed`, cause);
      }

      if (!response.ok) {
        throw new OpdsError(`OPDS request to ${config.catalogUrl} returned ${response.status}`);
      }

      const xml = await response.text();
      return parseOpdsFeed(xml, config.catalogUrl, config.id);
    },
  };
}
