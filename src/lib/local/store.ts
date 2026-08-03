/**
 * On-device store for imported local EPUBs.
 *
 * Unlike the aggregated {@link ../index/types.LibraryIndex} (which caches the
 * merged catalog for offline display), this store owns the *originals* the user
 * imported: the parsed {@link Book} metadata, the **raw EPUB bytes**, and the
 * extracted cover image — each keyed by the book's local id.
 *
 * It lives in its own IndexedDB database (`libro-local`) so it evolves
 * independently of the catalog index DB. Keeping the raw bytes is what lets the
 * P5 reader open a book fully offline: it will call {@link LocalStore.getFile}
 * with a book id — the browser analog of the blueprint's `get_book_file`.
 *
 * ## Cover resolution — the `localcover:` scheme
 * A cover is bytes in the store, not a URL. An imported {@link Book} carries
 * `coverUrl = "localcover:<id>"`; the UI resolves that to a temporary object URL
 * via {@link localCoverObjectUrl}. Anything else in `coverUrl` (an `http(s):`
 * cover from a connector) is used verbatim.
 *
 * The interface is abstracted so the unit layer uses {@link InMemoryLocalStore}
 * and never needs a real IndexedDB.
 */

import type { Book } from '../models';

/** A stored EPUB: its metadata, raw bytes, and optional extracted cover. */
export interface StoredEpub {
  book: Book;
  /** The original EPUB file bytes, kept verbatim for the reader. */
  file: Blob;
  /** Extracted cover image bytes, when the EPUB had a cover. */
  cover?: Blob;
}

/** The persistence contract for imported local books. */
export interface LocalStore {
  /** Upsert a stored EPUB, keyed by `entry.book.id`. */
  put(entry: StoredEpub): Promise<void>;
  /** Whether a book with this local id is already stored. */
  has(id: string): Promise<boolean>;
  /** The metadata for one stored book, or `undefined`. */
  getBook(id: string): Promise<Book | undefined>;
  /** Every stored book's metadata. Order is not guaranteed. */
  listBooks(): Promise<Book[]>;
  /** The raw EPUB bytes for a book (for the P5 reader), or `undefined`. */
  getFile(id: string): Promise<Blob | undefined>;
  /** The extracted cover bytes for a book, or `undefined`. */
  getCover(id: string): Promise<Blob | undefined>;
  /** Remove everything. */
  clear(): Promise<void>;
}

/** URL scheme marking a cover that lives in the {@link LocalStore}. */
export const LOCALCOVER_SCHEME = 'localcover:';

/** Build the `coverUrl` value for a stored cover. */
export function localCoverUrl(id: string): string {
  return `${LOCALCOVER_SCHEME}${id}`;
}

/** Whether a `coverUrl` refers to a stored local cover. */
export function isLocalCoverUrl(url: string | undefined): url is string {
  return typeof url === 'string' && url.startsWith(LOCALCOVER_SCHEME);
}

/**
 * Resolve a `localcover:<id>` URL to a temporary object URL an image element can
 * display.
 *
 * Returns `undefined` when the id is not a local-cover URL or has no stored
 * cover. The caller owns the returned URL and must `URL.revokeObjectURL` it when
 * the image is no longer shown (see `App.svelte`).
 */
export async function localCoverObjectUrl(
  store: LocalStore,
  coverUrl: string | undefined,
): Promise<string | undefined> {
  if (!isLocalCoverUrl(coverUrl)) return undefined;
  const id = coverUrl.slice(LOCALCOVER_SCHEME.length);
  const cover = await store.getCover(id);
  return cover ? URL.createObjectURL(cover) : undefined;
}

/**
 * In-memory {@link LocalStore} for the unit layer. Not persistent; clones on the
 * way in/out so callers can't mutate stored state by reference.
 */
export class InMemoryLocalStore implements LocalStore {
  readonly #store = new Map<string, StoredEpub>();

  async put(entry: StoredEpub): Promise<void> {
    this.#store.set(entry.book.id, { ...entry, book: structuredClone(entry.book) });
  }

  async has(id: string): Promise<boolean> {
    return this.#store.has(id);
  }

  async getBook(id: string): Promise<Book | undefined> {
    const entry = this.#store.get(id);
    return entry ? structuredClone(entry.book) : undefined;
  }

  async listBooks(): Promise<Book[]> {
    return [...this.#store.values()].map((entry) => structuredClone(entry.book));
  }

  async getFile(id: string): Promise<Blob | undefined> {
    return this.#store.get(id)?.file;
  }

  async getCover(id: string): Promise<Blob | undefined> {
    return this.#store.get(id)?.cover;
  }

  async clear(): Promise<void> {
    this.#store.clear();
  }
}

const DB_NAME = 'libro-local';
const DB_VERSION = 1;
const BOOKS = 'books';
const FILES = 'files';
const COVERS = 'covers';

/**
 * IndexedDB-backed {@link LocalStore} — the app runtime persistence for imports.
 *
 * Three object stores in one database, all keyed by the book's local id: `books`
 * (metadata), `files` (raw EPUB blobs), and `covers` (extracted cover blobs).
 * Blobs are stored directly; every modern browser's structured clone supports
 * `Blob` in IndexedDB.
 */
export class IdbLocalStore implements LocalStore {
  #db: Promise<IDBDatabase> | undefined;

  async put(entry: StoredEpub): Promise<void> {
    const db = await this.#open();
    await this.#write(db, [BOOKS, FILES, COVERS], (tx) => {
      tx.objectStore(BOOKS).put(entry.book, entry.book.id);
      tx.objectStore(FILES).put(entry.file, entry.book.id);
      if (entry.cover) tx.objectStore(COVERS).put(entry.cover, entry.book.id);
      else tx.objectStore(COVERS).delete(entry.book.id);
    });
  }

  async has(id: string): Promise<boolean> {
    return (await this.getBook(id)) !== undefined;
  }

  async getBook(id: string): Promise<Book | undefined> {
    return this.#read(BOOKS, (store) => store.get(id)) as Promise<Book | undefined>;
  }

  async listBooks(): Promise<Book[]> {
    return this.#read(BOOKS, (store) => store.getAll()) as Promise<Book[]>;
  }

  async getFile(id: string): Promise<Blob | undefined> {
    return this.#read(FILES, (store) => store.get(id)) as Promise<Blob | undefined>;
  }

  async getCover(id: string): Promise<Blob | undefined> {
    return this.#read(COVERS, (store) => store.get(id)) as Promise<Blob | undefined>;
  }

  async clear(): Promise<void> {
    const db = await this.#open();
    await this.#write(db, [BOOKS, FILES, COVERS], (tx) => {
      tx.objectStore(BOOKS).clear();
      tx.objectStore(FILES).clear();
      tx.objectStore(COVERS).clear();
    });
  }

  #open(): Promise<IDBDatabase> {
    if (!this.#db) {
      this.#db = new Promise<IDBDatabase>((resolve, reject) => {
        const req = indexedDB.open(DB_NAME, DB_VERSION);
        req.onupgradeneeded = () => {
          for (const name of [BOOKS, FILES, COVERS]) {
            if (!req.result.objectStoreNames.contains(name)) req.result.createObjectStore(name);
          }
        };
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      });
    }
    return this.#db;
  }

  #write(db: IDBDatabase, stores: string[], work: (tx: IDBTransaction) => void): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = db.transaction(stores, 'readwrite');
      work(tx);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  }

  async #read<T>(store: string, work: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
    const db = await this.#open();
    return new Promise<T>((resolve, reject) => {
      const req = work(db.transaction(store, 'readonly').objectStore(store));
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  }
}
