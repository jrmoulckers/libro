/**
 * Normalized domain model shared across every provider/connector.
 *
 * Providers translate their own API responses into these types so the rest of the
 * app only ever deals with one canonical shape. Keep this model provider-agnostic:
 * nothing here should assume a particular backend or transport.
 *
 * This is the browser TypeScript re-expression of the prior core model. It replaces
 * the earlier `library.ts` seed (LibraryItem/sortLibrary/countByKind), whose
 * sort/count coverage is folded into `sortBooks`/`countByMediaType` below.
 */

/**
 * The kind of media a {@link Book} represents.
 *
 * "Book" is the historical name for "a catalog item"; it also covers audiobooks and
 * podcasts so the aggregation layer can stay uniform.
 */
export type MediaType = 'ebook' | 'audiobook' | 'podcast';

/** All media types, in display order. */
export const MEDIA_TYPES: readonly MediaType[] = ['ebook', 'audiobook', 'podcast'];

/**
 * Reading/listening progress for an item.
 *
 * Intentionally small for the foundation; a later phase expands it with per-device
 * positions and a conflict-resolution strategy for device-to-device sync.
 */
export interface Progress {
  /** Fractional completion in the range `0..1`. */
  fraction: number;
  /** Last position in seconds (audio) or an opaque locator offset (text). */
  positionSeconds?: number;
  /**
   * Opaque text locator (e.g. an EPUB CFI / foliate locator) used to resume a
   * reading position precisely. Omitted for audio or when unknown.
   */
  locator?: string;
  /** Whether the user has marked the item finished. */
  finished: boolean;
}

/**
 * A single normalized catalog item.
 *
 * Every connector maps its native representation onto this shape. The `identifiers`
 * map holds cross-provider keys (ISBN, ASIN, …) that the aggregation layer uses to
 * de-duplicate the same title arriving from multiple providers.
 */
export interface Book {
  /**
   * Stable id **within the source provider** (not globally unique on its own;
   * combine with {@link Book.sourceProviderId} via {@link bookKey}).
   */
  id: string;
  title: string;
  authors: string[];
  mediaType: MediaType;
  /** The {@link Provider.id} of the connector this item came from. */
  sourceProviderId: string;
  series?: string;
  coverUrl?: string;
  /**
   * Short synopsis/description, when known. Often filled by a later metadata
   * enrichment pass rather than the source provider.
   */
  description?: string;
  /**
   * Subject/genre tags, when known. Like {@link Book.description}, usually filled
   * by the metadata enrichment pass (Open Library `subjects` / Google Books
   * `categories`) rather than the source connector.
   */
  subjects?: string[];
  /** Identifier scheme -> value, e.g. `{ isbn: "…", asin: "…" }`. */
  identifiers?: Record<string, string>;
  /** Optional progress; omitted when unknown or not yet synced. */
  progress?: Progress;
}

/**
 * Globally-unique key for a book: source provider id + provider-local id.
 *
 * Used as the primary key in the library index so two providers can expose an
 * item with the same local id without colliding.
 */
export function bookKey(book: Pick<Book, 'id' | 'sourceProviderId'>): string {
  return `${book.sourceProviderId}:${book.id}`;
}

/** Sort by first author, then title, using locale-aware comparison. */
export function sortBooks(books: readonly Book[]): Book[] {
  return [...books].sort(
    (a, b) => firstAuthor(a).localeCompare(firstAuthor(b)) || a.title.localeCompare(b.title),
  );
}

/** Count books of a single media type. */
export function countByMediaType(books: readonly Book[], mediaType: MediaType): number {
  return books.filter((book) => book.mediaType === mediaType).length;
}

/** Group books by media type, preserving the input order within each group. */
export function groupByMediaType(books: readonly Book[]): Map<MediaType, Book[]> {
  const groups = new Map<MediaType, Book[]>();
  for (const type of MEDIA_TYPES) {
    groups.set(type, []);
  }
  for (const book of books) {
    (groups.get(book.mediaType) ?? groups.set(book.mediaType, []).get(book.mediaType)!).push(book);
  }
  return groups;
}

/**
 * Merge a flat list of books, dropping duplicates of the same title arriving from
 * multiple providers.
 *
 * Dedup strategy, in order:
 *  1. Any shared normalized identifier (isbn, asin, …) means "same item".
 *  2. Otherwise fall back to a normalized `title|authors|mediaType` signature.
 *
 * The first occurrence wins; later duplicates are discarded. Order is otherwise
 * preserved, so callers can sort afterwards.
 */
export function dedupeBooks(books: readonly Book[]): Book[] {
  const seenById = new Map<string, number>();
  const out: Book[] = [];

  for (const book of books) {
    const keys = dedupeKeys(book);
    const hit = keys.map((k) => seenById.get(k)).find((i) => i !== undefined);

    if (hit !== undefined) {
      continue;
    }

    const index = out.push(book) - 1;
    for (const key of keys) {
      seenById.set(key, index);
    }
  }

  return out;
}

function dedupeKeys(book: Book): string[] {
  const keys: string[] = [];
  for (const [scheme, value] of Object.entries(book.identifiers ?? {})) {
    const normalized = value.trim().toLowerCase();
    if (normalized) {
      keys.push(`id:${scheme.trim().toLowerCase()}:${normalized}`);
    }
  }
  keys.push(`sig:${signature(book)}`);
  return keys;
}

function signature(book: Book): string {
  const authors = book.authors
    .map((a) => a.trim().toLowerCase())
    .filter(Boolean)
    .sort()
    .join(',');
  return `${book.title.trim().toLowerCase()}|${authors}|${book.mediaType}`;
}

function firstAuthor(book: Book): string {
  return book.authors[0] ?? '';
}
