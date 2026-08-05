/**
 * The persistence boundary for the aggregated catalog.
 *
 * The library index stores the merged {@link Book}[] on-device so the library
 * survives reloads and works offline. It is deliberately abstracted behind an
 * interface so the unit layer can use a fast in-memory fake ({@link
 * ./memory.InMemoryLibraryIndex}) while the app uses the real IndexedDB-backed
 * implementation ({@link ./idb.IdbLibraryIndex}).
 *
 * Keys are the globally-unique {@link ../models.bookKey} of each book
 * (`sourceProviderId:id`).
 *
 * Pure-client note: this is the *only* place library data lives — there is no
 * server copy. Phase-PWA's service worker layers offline caching on top of this;
 * it does not replace it.
 */

import type { Book } from '../models';

export interface LibraryIndex {
  /**
   * Upsert books into the index, keyed by {@link ../models.bookKey}. Replaces the
   * stored copy of any book with a matching key.
   */
  put(books: readonly Book[]): Promise<void>;

  /** Fetch a single book by its {@link ../models.bookKey}, or `undefined`. */
  get(key: string): Promise<Book | undefined>;

  /** Every stored book. Order is not guaranteed; callers sort as needed. */
  list(): Promise<Book[]>;

  /** Remove everything from the index. */
  clear(): Promise<void>;
}
