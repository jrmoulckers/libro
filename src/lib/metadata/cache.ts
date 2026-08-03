/**
 * On-device cache of resolved metadata patches, keyed by **ISBN**.
 *
 * Enrichment hits public APIs, so we persist every resolved {@link MetadataPatch}
 * — including *empty* (nothing-found) results — so the same ISBN is never fetched
 * twice across reloads. This keeps the app polite to Open Library / Google Books
 * and makes enrichment effectively free on subsequent launches (offline too).
 *
 * Abstracted behind an interface so the unit layer uses {@link InMemoryMetadataCache}
 * and never needs a real IndexedDB. The IndexedDB implementation lives in its own
 * `libro-metadata` database, independent of the catalog index and the other stores.
 */

import type { MetadataPatch } from './types';

/** The persistence contract for cached metadata patches. */
export interface MetadataCache {
  /** The cached patch for an ISBN, or `undefined` if never looked up. */
  get(isbn: string): Promise<MetadataPatch | undefined>;
  /** Cache the resolved patch for an ISBN (store `{}` for a negative result). */
  set(isbn: string, patch: MetadataPatch): Promise<void>;
  /** Every cached patch, keyed by ISBN. */
  all(): Promise<Map<string, MetadataPatch>>;
  /** Forget everything. */
  clear(): Promise<void>;
}

/** In-memory {@link MetadataCache} for the unit layer. Not persistent. */
export class InMemoryMetadataCache implements MetadataCache {
  readonly #store = new Map<string, MetadataPatch>();

  async get(isbn: string): Promise<MetadataPatch | undefined> {
    const patch = this.#store.get(isbn);
    return patch ? clonePatch(patch) : undefined;
  }

  async set(isbn: string, patch: MetadataPatch): Promise<void> {
    this.#store.set(isbn, clonePatch(patch));
  }

  async all(): Promise<Map<string, MetadataPatch>> {
    return new Map([...this.#store].map(([isbn, patch]) => [isbn, clonePatch(patch)]));
  }

  async clear(): Promise<void> {
    this.#store.clear();
  }
}

/** Deep-copy a patch so cached values can't be mutated through their arrays. */
function clonePatch(patch: MetadataPatch): MetadataPatch {
  return {
    ...patch,
    ...(patch.authors ? { authors: [...patch.authors] } : {}),
    ...(patch.subjects ? { subjects: [...patch.subjects] } : {}),
  };
}

const DB_NAME = 'libro-metadata';
const DB_VERSION = 1;
const STORE = 'patches';

/** IndexedDB-backed {@link MetadataCache} — the app runtime persistence. */
export class IdbMetadataCache implements MetadataCache {
  #db: Promise<IDBDatabase> | undefined;

  async get(isbn: string): Promise<MetadataPatch | undefined> {
    return this.#read((store) => store.get(isbn)) as Promise<MetadataPatch | undefined>;
  }

  async set(isbn: string, patch: MetadataPatch): Promise<void> {
    const db = await this.#open();
    await this.#write(db, (store) => store.put(patch, isbn));
  }

  async all(): Promise<Map<string, MetadataPatch>> {
    const db = await this.#open();
    const [keys, values] = await Promise.all([
      this.#request<IDBValidKey[]>(db, (store) => store.getAllKeys()),
      this.#request<MetadataPatch[]>(db, (store) => store.getAll()),
    ]);
    const map = new Map<string, MetadataPatch>();
    keys.forEach((key, i) => map.set(String(key), values[i]));
    return map;
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
