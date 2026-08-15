/**
 * On-device persistence for per-book **book-absolute** listening positions.
 *
 * Keyed by {@link ../models.Book.id}, this stores the whole-book position (in
 * seconds) the player writes on pause / chapter change / throttled during
 * playback, so an audiobook resumes where the user left off and the library can
 * show a listening percentage.
 *
 * It is the local analog of the blueprint's ListeningStore; P8 (progress sync)
 * will reconcile this device-local position with a remote tracker (Audiobookshelf
 * `PATCH /api/me/progress/{id}`).
 *
 * Abstracted behind an interface so the unit layer uses {@link InMemoryListeningStore}
 * and never needs a real IndexedDB. The IndexedDB implementation lives in its own
 * `libro-listening` database, independent of the catalog index, the imported-file
 * store, and the reading-position store.
 */

/** A persisted per-book listening position. */
export interface ListeningPosition {
  /** Book-absolute position in seconds over the unified multi-track timeline. */
  positionSeconds: number;
  /** Book-wide completion `0..1`, precomputed so cards need no timeline. */
  fraction: number;
  /** Whether the book is effectively finished. */
  finished: boolean;
}

/** The persistence contract for listening positions. */
export interface ListeningStore {
  /** The saved position for a book, or `undefined` if unheard. */
  get(bookId: string): Promise<ListeningPosition | undefined>;
  /** Upsert the position for a book. */
  set(bookId: string, position: ListeningPosition): Promise<void>;
  /** Every saved position, keyed by book id. */
  all(): Promise<Map<string, ListeningPosition>>;
  /** Forget a book's position. */
  remove(bookId: string): Promise<void>;
  /** Forget everything. */
  clear(): Promise<void>;
}

/** In-memory {@link ListeningStore} for the unit layer. Not persistent. */
export class InMemoryListeningStore implements ListeningStore {
  readonly #store = new Map<string, ListeningPosition>();

  async get(bookId: string): Promise<ListeningPosition | undefined> {
    const position = this.#store.get(bookId);
    return position ? { ...position } : undefined;
  }

  async set(bookId: string, position: ListeningPosition): Promise<void> {
    this.#store.set(bookId, { ...position });
  }

  async all(): Promise<Map<string, ListeningPosition>> {
    return new Map([...this.#store].map(([id, pos]) => [id, { ...pos }]));
  }

  async remove(bookId: string): Promise<void> {
    this.#store.delete(bookId);
  }

  async clear(): Promise<void> {
    this.#store.clear();
  }
}

const DB_NAME = 'libro-listening';
const DB_VERSION = 1;
const STORE = 'positions';

/** IndexedDB-backed {@link ListeningStore} — the app runtime persistence. */
export class IdbListeningStore implements ListeningStore {
  #db: Promise<IDBDatabase> | undefined;

  async get(bookId: string): Promise<ListeningPosition | undefined> {
    return this.#read((store) => store.get(bookId)) as Promise<ListeningPosition | undefined>;
  }

  async set(bookId: string, position: ListeningPosition): Promise<void> {
    const db = await this.#open();
    await this.#write(db, (store) => store.put(position, bookId));
  }

  async all(): Promise<Map<string, ListeningPosition>> {
    const db = await this.#open();
    const [keys, values] = await Promise.all([
      this.#request<IDBValidKey[]>(db, (store) => store.getAllKeys()),
      this.#request<ListeningPosition[]>(db, (store) => store.getAll()),
    ]);
    const map = new Map<string, ListeningPosition>();
    keys.forEach((key, i) => {
      const value = values[i];
      if (value !== undefined) map.set(String(key), value);
    });
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
