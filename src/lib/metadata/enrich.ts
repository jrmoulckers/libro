/**
 * Metadata enrichment orchestration — the post-aggregation pass that fills gaps
 * on the merged catalog from public bibliographic APIs.
 *
 * Design (ported from the blueprint's catalog enrichment, simplified to the
 * ISBN-only path this phase targets):
 *  - **Gap + ISBN gated** — only books that are missing an enrichable field *and*
 *    carry a usable ISBN are looked up; everything else passes through untouched.
 *  - **De-duplicated** — identical ISBNs across books collapse to one lookup.
 *  - **Cache first** — a cached patch (incl. an empty "nothing found" one) means
 *    no network at all; results are written back so the next run is free.
 *  - **Batched + fallback** — uncached ISBNs go through Open Library in batches,
 *    then Google Books per-ISBN fills whatever OL still left missing.
 *  - **Failure-isolated** — every lookup is guarded so one failing ISBN never
 *    aborts the pass; failures surface in `errors`. Non-clobbering: a resolved
 *    patch only ever fills gaps, never overwrites connector-provided data.
 *
 * All fetching is dependency-injected ({@link EnrichDeps}) so the unit layer runs
 * with fakes and **no real network**; {@link liveEnrichDeps} wires the real
 * CORS-open Open Library + Google Books shells for the app.
 */

import type { Book } from '../models';
import { fetchGoogleBooks } from './googlebooks';
import { fetchOpenLibrary } from './openlibrary';
import { IdbMetadataCache, InMemoryMetadataCache, type MetadataCache } from './cache';
import { idbAvailable } from '../index/idb';
import {
  bookGaps,
  combinePatches,
  extractIsbn,
  hasGaps,
  mergePatch,
  patchFields,
  type EnrichableField,
  type MetadataPatch,
} from './types';

/** A single failed lookup, isolated so it never aborts the pass. */
export interface EnrichError {
  isbn: string;
  source: string;
  reason: string;
}

/** Injected collaborators for {@link enrichBooks} (faked in tests). */
export interface EnrichDeps {
  cache: MetadataCache;
  /** Resolve many ISBNs at once (Open Library, batched). */
  fetchOpenLibrary: (isbns: string[]) => Promise<Map<string, MetadataPatch>>;
  /** Resolve one ISBN (Google Books, the fallback). */
  fetchGoogleBooks: (isbn: string) => Promise<MetadataPatch>;
  /** Max Google Books lookups in flight at once. Defaults to 4. */
  concurrency?: number;
}

export interface EnrichResult {
  books: Book[];
  errors: EnrichError[];
}

const DEFAULT_CONCURRENCY = 4;

/** Run `fn` over `items` with at most `limit` promises in flight. */
async function mapPool<T>(items: readonly T[], limit: number, fn: (item: T) => Promise<void>) {
  let cursor = 0;
  const size = Math.min(Math.max(1, limit), items.length || 1);
  const worker = async (): Promise<void> => {
    while (cursor < items.length) {
      const index = cursor++;
      const item = items[index];
      if (item === undefined) continue;
      await fn(item);
    }
  };
  await Promise.all(Array.from({ length: size }, worker));
}

/**
 * Enrich a batch of catalog books. Output preserves input order and length —
 * enrichment only augments, never adds or drops entries.
 */
export async function enrichBooks(books: readonly Book[], deps: EnrichDeps): Promise<EnrichResult> {
  const errors: EnrichError[] = [];

  // Phase 1: per-book ISBN, and the union of gaps per ISBN.
  const isbnByIndex: (string | null)[] = books.map((book) =>
    hasGaps(book) ? extractIsbn(book) : null,
  );
  const neededByIsbn = new Map<string, Set<EnrichableField>>();
  books.forEach((book, i) => {
    const isbn = isbnByIndex[i];
    if (!isbn) return;
    const set = neededByIsbn.get(isbn) ?? new Set<EnrichableField>();
    for (const field of bookGaps(book)) set.add(field);
    neededByIsbn.set(isbn, set);
  });

  const uniqueIsbns = [...neededByIsbn.keys()];
  if (uniqueIsbns.length === 0) return { books: [...books], errors };

  // Phase 2: split cached vs. to-fetch (a cached patch, even empty, skips network).
  const resolved = new Map<string, MetadataPatch>();
  const toFetch: string[] = [];
  for (const isbn of uniqueIsbns) {
    let cached: MetadataPatch | undefined;
    try {
      cached = await deps.cache.get(isbn);
    } catch {
      cached = undefined;
    }
    if (cached) resolved.set(isbn, cached);
    else toFetch.push(isbn);
  }

  // Phase 3: batch the uncached ISBNs through Open Library (failure-isolated).
  let olByIsbn = new Map<string, MetadataPatch>();
  if (toFetch.length > 0) {
    try {
      olByIsbn = await deps.fetchOpenLibrary(toFetch);
    } catch (error) {
      errors.push({
        isbn: toFetch.join(','),
        source: sourceOf(error, 'openlibrary'),
        reason: (error as Error).message,
      });
    }
  }

  // Phase 4: per-ISBN, fill whatever OL missed via Google Books, then cache.
  await mapPool(toFetch, deps.concurrency ?? DEFAULT_CONCURRENCY, async (isbn) => {
    const olPatch = olByIsbn.get(isbn) ?? {};
    const needed = neededByIsbn.get(isbn) ?? new Set<EnrichableField>();
    const supplied = patchFields(olPatch);
    const stillMissing = [...needed].filter((field) => !supplied.has(field));

    let combined = olPatch;
    if (stillMissing.length > 0) {
      try {
        combined = combinePatches(olPatch, await deps.fetchGoogleBooks(isbn));
      } catch (error) {
        errors.push({
          isbn,
          source: sourceOf(error, 'googlebooks'),
          reason: (error as Error).message,
        });
      }
    }

    resolved.set(isbn, combined);
    try {
      await deps.cache.set(isbn, combined);
    } catch {
      // A cache write failure is non-fatal; enrichment still applies this run.
    }
  });

  // Phase 5: apply patches in the original order, filling gaps only.
  const enriched = books.map((book, i) => {
    const isbn = isbnByIndex[i];
    const patch = isbn ? resolved.get(isbn) : undefined;
    return patch ? mergePatch(book, patch) : book;
  });

  return { books: enriched, errors };
}

function sourceOf(error: unknown, fallback: string): string {
  const source = (error as { source?: unknown })?.source;
  return typeof source === 'string' ? source : fallback;
}

/**
 * The real, CORS-open enrichment collaborators for the app: a persistent cache
 * plus the Open Library + Google Books fetch shells bound to the global `fetch`.
 */
export function liveEnrichDeps(
  cache: MetadataCache = idbAvailable() ? new IdbMetadataCache() : new InMemoryMetadataCache(),
): EnrichDeps {
  return {
    cache,
    fetchOpenLibrary: (isbns) => fetchOpenLibrary(isbns),
    fetchGoogleBooks: (isbn) => fetchGoogleBooks(isbn),
  };
}
