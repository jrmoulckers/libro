/**
 * EPUB file reader — the thin, zip-touching layer over the pure OPF parsers.
 *
 * An EPUB is a ZIP. This module unzips it and extracts just the few entries Libro
 * needs (`META-INF/container.xml`, the OPF, and the cover image), delegating all
 * XML parsing to the pure {@link ./opf} functions.
 *
 * ## Code-splitting (protects the bundle budget)
 * The zip reader (`fflate`) is pulled in with a **lazy `await import()`** so it
 * lands in its own chunk and never bloats the main entry — most sessions never
 * import a file. The blueprint calls this out explicitly; the CI `dist/` budget
 * (2048 KB) depends on it. Do not turn this into a static top-level import.
 *
 * ## No network / no CORS
 * Local import is the one connector with **no CORS concern**: everything happens
 * on bytes the user handed us via a file picker. Nothing is fetched or uploaded.
 *
 * The parsing logic lives in the pure functions, so this networked/zip surface is
 * kept thin and only lightly tested; the OPF/container behavior is covered by
 * `opf.test.ts`.
 */

import { parseContainerXml, parseOpf } from './opf';

/** Metadata extracted from an EPUB's OPF, before it becomes a {@link Book}. */
export interface EpubMetadata {
  title: string;
  authors: string[];
  description?: string;
  identifiers: Record<string, string>;
  series?: string;
}

/** The result of reading a single EPUB file. */
export interface ParsedEpub {
  metadata: EpubMetadata;
  /** Raw cover-image bytes, when the OPF declared a cover that exists in the zip. */
  coverBytes?: Uint8Array;
}

/** Typed error thrown when an EPUB cannot be read/parsed. */
export class EpubParseError extends Error {
  constructor(
    message: string,
    readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'EpubParseError';
  }
}

/**
 * Unzip an EPUB blob and extract its metadata + cover bytes. Lazily imports the
 * zip reader so it stays out of the entry chunk.
 *
 * @throws {EpubParseError} when the bytes are not a valid zip/EPUB or the
 * mandatory container/OPF entries are missing — callers isolate these per-file.
 */
export async function parseEpubFile(file: Blob): Promise<ParsedEpub> {
  const bytes = new Uint8Array(await file.arrayBuffer());

  // Lazy import => fflate is code-split into its own chunk (budget-critical).
  const { unzipSync, strFromU8 } = await import('fflate');

  let entries: Record<string, Uint8Array>;
  try {
    entries = unzipSync(bytes);
  } catch (cause) {
    throw new EpubParseError('not a valid EPUB (could not unzip)', cause);
  }

  const containerBytes = entries['META-INF/container.xml'];
  if (!containerBytes) throw new EpubParseError('missing META-INF/container.xml');

  const opfPath = parseContainerXml(strFromU8(containerBytes));
  if (!opfPath) throw new EpubParseError('no OPF rootfile in container.xml');

  const opfBytes = entries[opfPath];
  if (!opfBytes) throw new EpubParseError(`missing OPF package document at ${opfPath}`);

  const parsed = parseOpf(strFromU8(opfBytes), opfPath);
  const metadata: EpubMetadata = {
    title: parsed.title,
    authors: parsed.authors,
    description: parsed.description,
    identifiers: parsed.identifiers,
    series: parsed.series,
  };

  const coverBytes = parsed.coverHref ? entries[parsed.coverHref] : undefined;
  return { metadata, coverBytes };
}
