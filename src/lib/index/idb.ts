/**
 * IndexedDB-backed {@link LibraryIndex} — the app runtime persistence.
 *
 * A thin, dependency-free promise wrapper over IndexedDB. The aggregated catalog
 * is stored in a single object store keyed by {@link ../models.bookKey}, so the
 * library survives reloads and is available offline.
 *
 * Unit tests use {@link ./memory.InMemoryLibraryIndex} instead; this class is
 * exercised in the browser. Guard construction with {@link idbAvailable} where a
 * fallback matters (e.g. private-mode browsers that disable IndexedDB).
 */

import { bookKey, type Book } from '../models';
import type { LibraryIndex } from './types';

const DB_NAME = 'libro';
const DB_VERSION = 1;
const STORE = 'books';

/** Whether IndexedDB is usable in the current environment. */
export function idbAvailable(): boolean {
  return typeof indexedDB !== 'undefined';
}

export class IdbLibraryIndex implements LibraryIndex {
  #db: Promise<IDBDatabase> | undefined;

  constructor(
    private readonly dbName = DB_NAME,
    private readonly storeName = STORE,
  ) {}

  async put(books: readonly Book[]): Promise<void> {
    if (books.length === 0) return;
    const db = await this.#open();
    await this.#tx(db, 'readwrite', (store) => {
      for (const book of books) {
        store.put(book, bookKey(book));
      }
    });
  }

  async get(key: string): Promise<Book | undefined> {
    const db = await this.#open();
    return this.#request(db, 'readonly', (store) => store.get(key)) as Promise<Book | undefined>;
  }

  async list(): Promise<Book[]> {
    const db = await this.#open();
    return this.#request(db, 'readonly', (store) => store.getAll()) as Promise<Book[]>;
  }

  async clear(): Promise<void> {
    const db = await this.#open();
    await this.#tx(db, 'readwrite', (store) => store.clear());
  }

  #open(): Promise<IDBDatabase> {
    if (!this.#db) {
      this.#db = new Promise<IDBDatabase>((resolve, reject) => {
        const req = indexedDB.open(this.dbName, DB_VERSION);
        req.onupgradeneeded = () => {
          if (!req.result.objectStoreNames.contains(this.storeName)) {
            req.result.createObjectStore(this.storeName);
          }
        };
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      });
    }
    return this.#db;
  }

  /** Run a write transaction and resolve when it commits. */
  #tx(
    db: IDBDatabase,
    mode: IDBTransactionMode,
    work: (store: IDBObjectStore) => void,
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = db.transaction(this.storeName, mode);
      work(tx.objectStore(this.storeName));
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  }

  /** Run a single request and resolve with its result. */
  #request<T>(
    db: IDBDatabase,
    mode: IDBTransactionMode,
    work: (store: IDBObjectStore) => IDBRequest<T>,
  ): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const tx = db.transaction(this.storeName, mode);
      const req = work(tx.objectStore(this.storeName));
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  }
}
