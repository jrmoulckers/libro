/**
 * The declarative-manifest engine — interprets a {@link DeclarativePluginManifest}
 * as a {@link Provider}. Browser re-expression of the blueprint's `engine.rs`.
 *
 * `mapDeclarativeCatalog` applies the manifest's dotted-path field selectors over
 * a fetched JSON body to build normalized {@link Book}s; `checkDomainAllowed`
 * enforces the network sandbox (the resolved host must be on the manifest's
 * allowlist). Both are PURE — unit-tested from fixtures with no network.
 *
 * The only impure part is the deps-injected `fetch` inside
 * {@link createDeclarativePluginProvider}: the resolved URL is sandbox-checked
 * **before** any request, so a denied domain is unreachable (a typed
 * {@link PluginError}, never a fetch).
 *
 * ## CORS reality
 * Like the OPDS/Audiobookshelf connectors, a declarative plugin fetches the
 * third-party server directly from the browser (no app-owned proxy — studio rule:
 * no server tier). It therefore only works against a server that returns
 * permissive `Access-Control-Allow-Origin`. A blocked fetch surfaces as a failed
 * provider in `aggregateLibrary`'s `Promise.allSettled`, never crashing the app.
 */

import type { Book } from '../models';
import type { Provider, ProviderCapability } from '../providers/types';
import type { DeclarativePluginManifest } from './manifest';
import { PluginError } from './manifest';

/** Deps for {@link createDeclarativePluginProvider} — the network seam. */
export interface DeclarativePluginDeps {
  /**
   * Fetch the catalog URL and return parsed JSON. Deps-injected so tests replay
   * fixtures with no network. Defaults to `fetch` + `response.json()`.
   */
  fetchJson?: (url: string) => Promise<unknown>;
}

/**
 * Enforce the network sandbox: `url` must be an http(s) URL whose host equals or
 * is a subdomain of an allowlisted bare host. PURE; returns a boolean (never
 * throws). Handles scheme, userinfo, port, subdomain, and case edge cases.
 */
export function checkDomainAllowed(url: string, allowedDomains: readonly string[]): boolean {
  const host = hostOf(url);
  if (!host) return false;
  return allowedDomains.some((domain) => hostMatchesDomain(host, domain));
}

/** Extract the lowercased host from an http(s) URL, dropping userinfo and port. */
function hostOf(url: string): string | null {
  let rest: string;
  if (url.startsWith('https://')) rest = url.slice('https://'.length);
  else if (url.startsWith('http://')) rest = url.slice('http://'.length);
  else return null;

  const end = rest.search(/[/?#]/);
  const authority = end === -1 ? rest : rest.slice(0, end);
  // Drop any userinfo (`user:pass@host`) then any port.
  const afterUserinfo = authority.includes('@')
    ? authority.slice(authority.lastIndexOf('@') + 1)
    : authority;
  const host = afterUserinfo.split(':')[0];
  return host ? host.toLowerCase() : null;
}

/** A host matches a domain if it equals it or is a subdomain of it. */
function hostMatchesDomain(host: string, domain: string): boolean {
  const d = domain.trim().toLowerCase();
  if (!d) return false;
  return host === d || host.endsWith(`.${d}`);
}

/**
 * Map a fetched JSON body to normalized {@link Book}s using the manifest's field
 * selectors. PURE + robust: a non-array items location or a non-object /
 * id-less / title-less item is skipped, never fatal — one bad record can't sink
 * the whole catalog. Every book is stamped with `manifest.id` as its source.
 */
export function mapDeclarativeCatalog(json: unknown, manifest: DeclarativePluginManifest): Book[] {
  const { catalog } = manifest;
  const items = catalog.itemsPath
    ? (asArray(getPath(json, catalog.itemsPath)) ?? [])
    : (asArray(json) ?? []);

  const f = catalog.fields;
  const mediaType = catalog.mediaType ?? 'ebook';
  const books: Book[] = [];

  for (const item of items) {
    if (!isRecord(item)) continue;
    const id = getString(item, f.id);
    const title = getString(item, f.title);
    if (!id || !title) continue;

    const book: Book = {
      id,
      title,
      authors: f.authors ? extractAuthors(getPath(item, f.authors)) : [],
      mediaType,
      sourceProviderId: manifest.id,
    };

    if (f.series) {
      const series = getString(item, f.series);
      if (series) book.series = series;
    }
    if (f.cover) {
      const cover = getString(item, f.cover);
      if (cover) book.coverUrl = cover;
    }
    if (f.description) {
      const description = getString(item, f.description);
      if (description) book.description = description;
    }
    if (f.identifiers) {
      const identifiers = extractIdentifiers(getPath(item, f.identifiers));
      if (Object.keys(identifiers).length > 0) book.identifiers = identifiers;
    }

    books.push(book);
  }

  return books;
}

/**
 * A {@link Provider} driven entirely by a declarative manifest (no plugin code).
 * `listBooks()` sandbox-checks the resolved URL, fetches it (deps-injected), and
 * maps the response via {@link mapDeclarativeCatalog}. A denied domain throws a
 * typed {@link PluginError} **before** any fetch.
 */
export function createDeclarativePluginProvider(
  manifest: DeclarativePluginManifest,
  deps: DeclarativePluginDeps = {},
): Provider {
  const capabilities: ReadonlySet<ProviderCapability> = new Set(manifest.capabilities);
  const url = joinUrl(manifest.baseUrl, manifest.catalog.endpoint);
  const fetchJson = deps.fetchJson ?? defaultFetchJson;

  return {
    id: manifest.id,
    displayName: manifest.displayName,
    capabilities,
    async listBooks(): Promise<Book[]> {
      // SANDBOX: the resolved host must be allowlisted — checked before any fetch.
      if (!checkDomainAllowed(url, manifest.allowedDomains)) {
        throw new PluginError(
          'domain-denied',
          `network permission denied: '${url}' is not in allowedDomains`,
        );
      }
      let json: unknown;
      try {
        json = await fetchJson(url);
      } catch (cause) {
        throw new PluginError('http', `plugin '${manifest.id}' request to ${url} failed`, cause);
      }
      return mapDeclarativeCatalog(json, manifest);
    },
  };
}

async function defaultFetchJson(url: string): Promise<unknown> {
  const response = await fetch(url, { headers: { Accept: 'application/json' } });
  if (!response.ok) {
    throw new PluginError('http', `request to ${url} returned ${response.status}`);
  }
  return response.json();
}

/** Join a base URL and an endpoint path, normalizing the slash between them. */
export function joinUrl(baseUrl: string, endpoint: string): string {
  const base = baseUrl.replace(/\/+$/, '');
  if (!endpoint) return base;
  return endpoint.startsWith('/') ? `${base}${endpoint}` : `${base}/${endpoint}`;
}

/** Extract authors: an array of scalars, or a single comma-separated string. */
export function extractAuthors(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .map(valueToString)
      .filter((s): s is string => Boolean(s))
      .map((s) => s.trim())
      .filter(Boolean);
  }
  const single = valueToString(value);
  if (single) {
    return single
      .split(',')
      .map((a) => a.trim())
      .filter(Boolean);
  }
  return [];
}

/**
 * Extract an identifiers record: an object of `scheme -> scalar` (each copied),
 * or a single scalar stored under `isbn`.
 */
function extractIdentifiers(value: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  if (isRecord(value)) {
    for (const [scheme, raw] of Object.entries(value)) {
      const v = valueToString(raw);
      if (v && v.trim()) out[scheme] = v.trim();
    }
    return out;
  }
  const scalar = valueToString(value);
  if (scalar && scalar.trim()) out.isbn = scalar.trim();
  return out;
}

/** Traverse a dotted path (`a.b.c`) into a JSON value. */
function getPath(value: unknown, path: string): unknown {
  let cur: unknown = value;
  for (const seg of path.split('.')) {
    if (!isRecord(cur)) return undefined;
    cur = cur[seg];
  }
  return cur;
}

/** Resolve a dotted path to a scalar string (string/number/bool coerced). */
function getString(item: unknown, path: string): string | undefined {
  return valueToString(getPath(item, path)) || undefined;
}

/** Coerce a scalar JSON value to a string. Objects/arrays/null -> undefined. */
function valueToString(v: unknown): string | undefined {
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function asArray(value: unknown): unknown[] | undefined {
  return Array.isArray(value) ? value : undefined;
}
