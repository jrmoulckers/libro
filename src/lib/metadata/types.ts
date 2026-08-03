/**
 * Shared types + pure helpers for the metadata-enrichment layer.
 *
 * Enrichment is a **distinct concern** from library {@link ../providers Provider}s:
 * a provider answers "what does the user own?"; a metadata source answers "what is
 * the canonical data for this title?" (cover, description, authors, series,
 * subjects). It never takes part in the aggregation fan-out. The enrichment pass
 * only ever *fills gaps* on already-aggregated books — it never overwrites data a
 * connector already provided.
 *
 * Everything here is pure (no network, no DOM) so it is the primary unit-tested
 * surface; the actual `fetch` lives in the thin shells (`openlibrary.ts` /
 * `googlebooks.ts`) and is dependency-injected into {@link ../metadata/enrich}.
 */

import type { Book } from '../models';

/**
 * Error thrown by a metadata fetch shell on a transport failure, non-OK HTTP
 * status, or malformed response. A *missing* record (an empty result) is **not**
 * an error — it is an empty {@link MetadataPatch}. `enrichBooks` isolates these so
 * one failing lookup never aborts the pass or the library render.
 */
export class MetadataError extends Error {
  constructor(
    message: string,
    /** The metadata source id that failed, e.g. `"openlibrary"`. */
    readonly source: string,
  ) {
    super(message);
    this.name = 'MetadataError';
  }
}

/**
 * A gap-filling patch resolved from a public bibliographic API. Every field is
 * optional: a source that lacks a field simply omits it, and a completely empty
 * patch is a valid ("nothing found") result that still gets cached so the same
 * ISBN is never re-fetched.
 */
export interface MetadataPatch {
  description?: string;
  coverUrl?: string;
  authors?: string[];
  series?: string;
  subjects?: string[];
}

/** The {@link Book} fields the enrichment pass is able to fill. */
export type EnrichableField = 'description' | 'coverUrl' | 'authors' | 'series' | 'subjects';

/**
 * Normalize a raw identifier value to a canonical ISBN-10/13, or `null` if it is
 * not a well-formed ISBN. Strips hyphens/spaces and upper-cases a trailing `X`;
 * an ISBN-13 must start with the `978`/`979` Bookland prefix.
 */
export function normalizeIsbn(raw: string): string | null {
  const cleaned = raw.replace(/[^0-9Xx]/g, '').toUpperCase();
  return /^(?:97[89]\d{10}|\d{9}[\dX])$/.test(cleaned) ? cleaned : null;
}

// Identifier keys that plausibly carry an ISBN. We accept the common connector
// spellings (`isbn`, `isbn_13`, `isbn10`, …) plus the OPDS/Dublin-Core schemes,
// where ISBNs arrive as `urn:isbn:…`. Restricting by key avoids mistaking an
// unrelated 13-digit identifier for an ISBN.
const ISBN_KEY = /isbn/i;

/**
 * The best ISBN to enrich a book by, or `null` when it has none. Prefers a
 * 13-digit ISBN over a 10-digit one (unambiguous, and what both APIs key on).
 */
export function extractIsbn(book: Pick<Book, 'identifiers'>): string | null {
  const candidates: string[] = [];
  for (const [key, value] of Object.entries(book.identifiers ?? {})) {
    const k = key.toLowerCase();
    if (ISBN_KEY.test(k) || k.startsWith('dcterms:') || k.startsWith('opds:')) {
      const normalized = normalizeIsbn(value);
      if (normalized) candidates.push(normalized);
    }
  }
  return candidates.find((c) => c.length === 13) ?? candidates.find((c) => c.length === 10) ?? null;
}

/** Whether a field on `book` is empty and therefore enrichable. */
function isGap(book: Book, field: EnrichableField): boolean {
  switch (field) {
    case 'authors':
      return (book.authors?.length ?? 0) === 0;
    case 'subjects':
      return (book.subjects?.length ?? 0) === 0;
    default:
      return !book[field];
  }
}

/** The set of enrichable fields currently missing from `book`. */
export function bookGaps(book: Book): Set<EnrichableField> {
  const fields: EnrichableField[] = ['description', 'coverUrl', 'authors', 'series', 'subjects'];
  return new Set(fields.filter((f) => isGap(book, f)));
}

/** Whether `book` is missing at least one field enrichment could fill. */
export function hasGaps(book: Book): boolean {
  return bookGaps(book).size > 0;
}

/**
 * Overlay `fallback` onto `primary`, keeping every non-empty `primary` field and
 * only borrowing what it lacks. Used to combine an Open Library patch (primary)
 * with a Google Books patch (fallback, esp. for the description OL often omits).
 */
export function combinePatches(primary: MetadataPatch, fallback: MetadataPatch): MetadataPatch {
  const patch: MetadataPatch = {};
  const description = primary.description ?? fallback.description;
  if (description) patch.description = description;
  const coverUrl = primary.coverUrl ?? fallback.coverUrl;
  if (coverUrl) patch.coverUrl = coverUrl;
  const authors = primary.authors?.length ? primary.authors : fallback.authors;
  if (authors?.length) patch.authors = [...authors];
  const series = primary.series ?? fallback.series;
  if (series) patch.series = series;
  const subjects = primary.subjects?.length ? primary.subjects : fallback.subjects;
  if (subjects?.length) patch.subjects = [...subjects];
  return patch;
}

/** Which enrichable fields a patch can actually supply (non-empty). */
export function patchFields(patch: MetadataPatch): Set<EnrichableField> {
  const fields = new Set<EnrichableField>();
  if (patch.description) fields.add('description');
  if (patch.coverUrl) fields.add('coverUrl');
  if (patch.authors?.length) fields.add('authors');
  if (patch.series) fields.add('series');
  if (patch.subjects?.length) fields.add('subjects');
  return fields;
}

/**
 * Return a **new** book with `patch` filling only the gaps — connector-provided
 * values are never clobbered. Returns the same reference when nothing changes so
 * callers can cheaply detect no-ops.
 */
export function mergePatch(book: Book, patch: MetadataPatch): Book {
  const gaps = bookGaps(book);
  if (gaps.size === 0) return book;

  const next: Book = { ...book };
  let changed = false;
  if (gaps.has('authors') && patch.authors?.length) {
    next.authors = [...patch.authors];
    changed = true;
  }
  if (gaps.has('coverUrl') && patch.coverUrl) {
    next.coverUrl = patch.coverUrl;
    changed = true;
  }
  if (gaps.has('description') && patch.description) {
    next.description = patch.description;
    changed = true;
  }
  if (gaps.has('series') && patch.series) {
    next.series = patch.series;
    changed = true;
  }
  if (gaps.has('subjects') && patch.subjects?.length) {
    next.subjects = [...patch.subjects];
    changed = true;
  }
  return changed ? next : book;
}
