/**
 * Local-file import orchestration + the {@link Provider} that surfaces imported
 * books in the aggregated library.
 *
 * This is the browser re-expression of the blueprint's `localfiles` connector.
 * The blueprint recursively *scanned* folders on disk; a pure-client app cannot,
 * so here the **user picks files** (see `App.svelte`) and we import the bytes they
 * hand us. There is no filesystem scan and no directory-access assumption.
 *
 * Flow: for each picked file we parse its EPUB metadata (via the lazily-loaded
 * {@link ../epub/epub.parseEpubFile}), build a normalized {@link Book}, and
 * persist the metadata + raw bytes + cover into the {@link LocalStore}. The
 * {@link createLocalProvider} reads that store back with **no network**, so
 * imported books merge into the catalog alongside connector books.
 *
 * Robustness mirrors the blueprint: a non-EPUB, unparseable, or already-imported
 * file is skipped *with a reason* — never a thrown batch. One bad file can't
 * abort the import.
 */

import type { Book } from '../models';
import type { Provider, ProviderCapability } from '../providers/types';
import { parseEpubFile, type ParsedEpub } from '../epub/epub';
import { localCoverUrl, type LocalStore } from './store';

/** The provider id used as {@link Book.sourceProviderId} for imported files. */
export const LOCAL_PROVIDER_ID = 'localfiles';

/** A file that was skipped during import, with a human-readable reason. */
export interface ImportSkip {
  name: string;
  reason: string;
}

/** The outcome of an import batch. */
export interface ImportResult {
  imported: Book[];
  errors: ImportSkip[];
}

/** Injectable EPUB parser, so tests can run offline without a real zip. */
export type EpubParser = (file: Blob) => Promise<ParsedEpub>;

/** A pickable file: the subset of `File` this module needs. */
export interface ImportableFile extends Blob {
  readonly name: string;
}

/**
 * Stable local id for a picked file. Mirrors the blueprint's path-hash id with a
 * browser-appropriate key: a hash of `name + size` (no filesystem path exists).
 * Deterministic, so re-importing the same file dedupes to the same id.
 */
export function localBookId(file: ImportableFile): string {
  return `local-${fnv1a(`${file.name}:${file.size}`)}`;
}

/**
 * Build a normalized {@link Book} from parsed EPUB metadata. Falls back to the
 * filename (sans extension) when the OPF had no title, mirroring the blueprint.
 */
export function buildLocalBook(id: string, file: ImportableFile, parsed: ParsedEpub): Book {
  const identifiers: Record<string, string> = {
    ...parsed.metadata.identifiers,
    'local:source': file.name,
  };

  const title = parsed.metadata.title.trim() || stripExtension(file.name);

  const book: Book = {
    id,
    title,
    authors: parsed.metadata.authors,
    mediaType: 'ebook',
    sourceProviderId: LOCAL_PROVIDER_ID,
    identifiers,
  };
  if (parsed.metadata.series) book.series = parsed.metadata.series;
  if (parsed.metadata.description) book.description = parsed.metadata.description;
  if (parsed.coverBytes) book.coverUrl = localCoverUrl(id);

  return book;
}

/**
 * Parse, dedupe and persist a batch of picked EPUB files.
 *
 * `parse` is injectable purely for testing; production always uses
 * {@link parseEpubFile}. Non-EPUB, unparseable, and already-imported files are
 * reported in `errors` and skipped; everything else is persisted and returned in
 * `imported`.
 */
export async function importEpubFiles(
  files: Iterable<ImportableFile>,
  store: LocalStore,
  parse: EpubParser = parseEpubFile,
): Promise<ImportResult> {
  const imported: Book[] = [];
  const errors: ImportSkip[] = [];
  const seenThisBatch = new Set<string>();

  for (const file of files) {
    if (!isEpubName(file.name)) {
      errors.push({ name: file.name, reason: 'Not an .epub file' });
      continue;
    }

    const id = localBookId(file);
    if (seenThisBatch.has(id) || (await store.has(id))) {
      errors.push({ name: file.name, reason: 'Already in your library' });
      continue;
    }

    let parsed: ParsedEpub;
    try {
      parsed = await parse(file);
    } catch (cause) {
      errors.push({ name: file.name, reason: reasonFrom(cause) });
      continue;
    }

    const book = buildLocalBook(id, file, parsed);
    const cover = parsed.coverBytes
      ? new Blob([toArrayBuffer(parsed.coverBytes)], { type: 'application/octet-stream' })
      : undefined;

    try {
      await store.put({ book, file, cover });
    } catch (cause) {
      errors.push({ name: file.name, reason: reasonFrom(cause) });
      continue;
    }

    seenThisBatch.add(id);
    imported.push(book);
  }

  return { imported, errors };
}

/**
 * A {@link Provider} that lists the user's imported local books from the
 * {@link LocalStore}. Capability: `catalog` only. No network — safe to register
 * in the default app pipeline (see `library.ts`).
 */
export function createLocalProvider(store: LocalStore): Provider {
  const capabilities: ReadonlySet<ProviderCapability> = new Set(['catalog']);
  return {
    id: LOCAL_PROVIDER_ID,
    displayName: 'Local Files',
    capabilities,
    listBooks(): Promise<Book[]> {
      return store.listBooks();
    },
  };
}

function isEpubName(name: string): boolean {
  return name.toLowerCase().endsWith('.epub');
}

function stripExtension(name: string): string {
  const slash = Math.max(name.lastIndexOf('/'), name.lastIndexOf('\\'));
  const base = slash >= 0 ? name.slice(slash + 1) : name;
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(0, dot) : base;
}

function reasonFrom(cause: unknown): string {
  return cause instanceof Error ? cause.message : 'could not be read';
}

/** Copy into a standalone ArrayBuffer so `Blob` gets a clean, typed backing. */
function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  return bytes.slice().buffer;
}

/** FNV-1a (32-bit) hex hash — deterministic, dependency-free stable ids. */
function fnv1a(input: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < input.length; i += 1) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, '0');
}
