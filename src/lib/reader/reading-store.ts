/**
 * On-device persistence for per-book reading positions.
 *
 * Keyed by {@link ../models.Book.id}, this stores the lightweight
 * {@link ReadingPosition} the reader writes on chapter change / scroll, so a book
 * reopens where the user left off and the library can show a reading percentage.
 *
 * It is the local analog of the blueprint's ReadingStore; P8 (progress sync) will
 * build on it, reconciling this device-local position with a remote tracker.
 *
 * Abstracted behind an interface so the unit layer uses {@link InMemoryReadingStore}
 * and never needs a real IndexedDB. The IndexedDB implementation lives in its own
 * `libro-reading` database, independent of the catalog index and the imported-file
 * store.
 */

import type { ReadingPosition } from './locator';

/** The persistence contract for reading positions. */
export interface ReadingStore {
  /** The saved position for a book, or `undefined` if unread. */
  get(bookId: string): Promise<ReadingPosition | undefined>;
  /** Upsert the position for a book. */
  set(bookId: string, position: ReadingPosition): Promise<void>;
  /** Every saved position, keyed by book id. */
  all(): Promise<Map<string, ReadingPosition>>;
  /** Forget a book's position. */
  remove(bookId: string): Promise<void>;
  /** Forget everything. */
  clear(): Promise<void>;
}

/** In-memory {@link ReadingStore} for the unit layer. Not persistent. */
export class InMemoryReadingStore implements ReadingStore {
  readonly #store = new Map<string, ReadingPosition>();

  async get(bookId: string): Promise<ReadingPosition | undefined> {
    const position = this.#store.get(bookId);
    return position ? { ...position } : undefined;
  }

  async set(bookId: string, position: ReadingPosition): Promise<void> {
    this.#store.set(bookId, { ...position });
  }

  async all(): Promise<Map<string, ReadingPosition>> {
    return new Map([...this.#store].map(([id, pos]) => [id, { ...pos }]));
  }

  async remove(bookId: string): Promise<void> {
    this.#store.delete(bookId);
  }

  async clear(): Promise<void> {
    this.#store.clear();
  }
}

const DB_NAME = 'libro-reading';
const DB_VERSION = 1;
const STORE = 'positions';

/** IndexedDB-backed {@link ReadingStore} — the app runtime persistence. */
export class IdbReadingStore implements ReadingStore {
  #db: Promise<IDBDatabase> | undefined;

  async get(bookId: string): Promise<ReadingPosition | undefined> {
    return this.#read((store) => store.get(bookId)) as Promise<ReadingPosition | undefined>;
  }

  async set(bookId: string, position: ReadingPosition): Promise<void> {
    const db = await this.#open();
    await this.#write(db, (store) => store.put(position, bookId));
  }

  async all(): Promise<Map<string, ReadingPosition>> {
    const db = await this.#open();
    const [keys, values] = await Promise.all([
      this.#request<IDBValidKey[]>(db, (store) => store.getAllKeys()),
      this.#request<ReadingPosition[]>(db, (store) => store.getAll()),
    ]);
    const map = new Map<string, ReadingPosition>();
    keys.forEach((key, i) => map.set(String(key), values[i]));
    return map;
  }

  async remove(bookId: string): Promise<void> {
    const db = await this.#open();
    await this.#write(db, (store) => store.delete(bookId));
  }

  async clear(): Promise<void> {
    const db = await this.#open();
    await this.#write(db, (store) => store.clear());
  }

  #open(): Promise<IDBDatabase> {
    if (!this.#db) {
      this.#db = new Promise<IDBDatabase>((resolve, reject) => {
        const req = indexedDB.open(DB_NAME, DB_VERSION);
        req.onupgradeneeded = () => {
          if (!req.result.objectStoreNames.contains(STORE)) req.result.createObjectStore(STORE);
        };
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      });
    }
    return this.#db;
  }

  #write(db: IDBDatabase, work: (store: IDBObjectStore) => void): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, 'readwrite');
      work(tx.objectStore(STORE));
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  }

  async #read<T>(work: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
    const db = await this.#open();
    return this.#request(db, work);
  }

  #request<T>(db: IDBDatabase, work: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const req = work(db.transaction(STORE, 'readonly').objectStore(STORE));
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  }
}
