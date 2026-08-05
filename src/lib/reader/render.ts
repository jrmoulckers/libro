/**
 * Lazy EPUB render shell — the thin, zip-touching layer the reader mounts.
 *
 * This is the only reader module that touches the zip and the browser's object-
 * URL machinery; everything it depends on for *logic* (spine/TOC assembly, path
 * resolution, HTML rewriting) is a pure, separately-tested function. It is loaded
 * with a lazy `await import()` from `Reader.svelte`, and it itself lazily imports
 * `fflate`, so both the reader UI and the zip reader land in their own chunks and
 * never bloat the main entry (budget-critical — see the vite chunk report).
 *
 * Flow: unzip the stored EPUB bytes once, resolve `container.xml -> OPF ->
 * spine + TOC`, then render one spine document at a time into an iframe `srcdoc`
 * with its images/styles rewritten to object URLs extracted from the zip.
 *
 * In-house on purpose: rather than pull in a heavy reader library (epub.js /
 * react-reader, as the blueprint did), we reuse the `fflate` we already ship for
 * import plus our own OPF parser. That keeps the dependency surface and the bundle
 * small while covering the phase's needs.
 */

import { parseContainerXml, parseOpf } from '../epub/opf';
import {
  assembleSpine,
  parseNavDoc,
  parseNcx,
  spineIndexForHref,
  type EpubStructure,
  type ManifestItem,
} from '../epub/spine';
import { rewriteChapterHtml } from './rewrite';

/** A TOC chapter resolved to a spine index, ready to jump to. */
export interface ReaderChapter {
  title: string;
  /** Spine index this entry points at. */
  index: number;
}

/** A rendered chapter document + a cleanup handle for its object URLs. */
export interface RenderedChapter {
  srcdoc: string;
  /** Revoke every object URL this chapter minted. Call when leaving the chapter. */
  revoke: () => void;
}

/** An opened EPUB, ready to render chapter-by-chapter. */
export interface OpenedEpub {
  title: string;
  spineCount: number;
  /** Flat, jump-able table of contents (may be empty). */
  toc: ReaderChapter[];
  /** A human label for a spine index (TOC title when known, else "Chapter N"). */
  chapterLabel(index: number): string;
  /** Render the spine document at `index` into an iframe `srcdoc`. */
  renderChapter(index: number): RenderedChapter;
}

/**
 * Unzip and open an EPUB for reading. Lazily imports `fflate`.
 *
 * @throws when the bytes are not a valid EPUB or have an empty spine.
 */
export async function openEpub(bytes: Uint8Array): Promise<OpenedEpub> {
  const { unzipSync, strFromU8 } = await import('fflate');

  let entries: Record<string, Uint8Array>;
  try {
    entries = unzipSync(bytes);
  } catch (cause) {
    throw new Error('not a valid EPUB (could not unzip)', { cause });
  }

  const container = entries['META-INF/container.xml'];
  if (!container) throw new Error('missing META-INF/container.xml');
  const opfPath = parseContainerXml(strFromU8(container));
  if (!opfPath) throw new Error('no OPF rootfile in container.xml');
  const opfBytes = entries[opfPath];
  if (!opfBytes) throw new Error(`missing OPF at ${opfPath}`);
  const opfXml = strFromU8(opfBytes);

  const structure = assembleSpine(opfXml, opfPath);
  if (structure.spine.length === 0) throw new Error('EPUB has no readable spine');

  const title = parseOpf(opfXml, opfPath).title || 'Untitled';
  const toc = buildToc(structure, entries, strFromU8);
  const titleByIndex = new Map(toc.map((entry) => [entry.index, entry.title]));
  const mimeByHref = new Map(
    [...structure.manifest.values()].map((item: ManifestItem) => [item.href, item.mediaType]),
  );

  return {
    title,
    spineCount: structure.spine.length,
    toc,
    chapterLabel(index: number): string {
      return titleByIndex.get(index) ?? `Chapter ${index + 1}`;
    },
    renderChapter(index: number): RenderedChapter {
      const item = structure.spine[clampIndex(index, structure.spine.length)];
      const html = strFromU8(entries[item.href] ?? new Uint8Array());

      const created: string[] = [];
      const cache = new Map<string, string>();
      const resolve = (inZip: string): string | undefined => {
        const cached = cache.get(inZip);
        if (cached) return cached;
        const data = entries[inZip];
        if (!data) return undefined;
        const url = URL.createObjectURL(
          new Blob([toArrayBuffer(data)], {
            type: mimeByHref.get(inZip) || 'application/octet-stream',
          }),
        );
        cache.set(inZip, url);
        created.push(url);
        return url;
      };

      return {
        srcdoc: rewriteChapterHtml(html, item.href, resolve),
        revoke: () => {
          for (const url of created) URL.revokeObjectURL(url);
        },
      };
    },
  };
}

/** Read + parse the TOC document, mapping each entry onto a spine index. */
function buildToc(
  structure: EpubStructure,
  entries: Record<string, Uint8Array>,
  strFromU8: (data: Uint8Array) => string,
): ReaderChapter[] {
  if (!structure.toc) return [];
  const bytes = entries[structure.toc.href];
  if (!bytes) return [];

  const raw = strFromU8(bytes);
  const parsed =
    structure.toc.kind === 'nav'
      ? parseNavDoc(raw, structure.toc.href)
      : parseNcx(raw, structure.toc.href);

  const chapters: ReaderChapter[] = [];
  const seen = new Set<number>();
  for (const entry of parsed) {
    const index = spineIndexForHref(structure.spine, entry.href);
    if (index < 0 || seen.has(index)) continue;
    seen.add(index);
    chapters.push({ title: entry.title, index });
  }
  return chapters;
}

function clampIndex(index: number, length: number): number {
  return Math.min(Math.max(index, 0), Math.max(length - 1, 0));
}

/** Copy into a standalone ArrayBuffer so `Blob` gets a clean, typed backing. */
function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  return bytes.slice().buffer;
}
