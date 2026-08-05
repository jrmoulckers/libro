/**
 * In-memory {@link LibraryIndex} implementation.
 *
 * Used by the unit test layer so tests don't need a real IndexedDB (and so we
 * avoid pulling in a fake-indexeddb devDependency). Also a reasonable fallback for
 * environments without IndexedDB. Not persistent — everything is lost when the
 * instance is dropped.
 */

import { bookKey, type Book } from '../models';
import type { LibraryIndex } from './types';

export class InMemoryLibraryIndex implements LibraryIndex {
  readonly #store = new Map<string, Book>();

  async put(books: readonly Book[]): Promise<void> {
    for (const book of books) {
      this.#store.set(bookKey(book), structuredClone(book));
    }
  }

  async get(key: string): Promise<Book | undefined> {
    const book = this.#store.get(key);
    return book ? structuredClone(book) : undefined;
  }

  async list(): Promise<Book[]> {
    return [...this.#store.values()].map((book) => structuredClone(book));
  }

  async clear(): Promise<void> {
    this.#store.clear();
  }
}
