/**
 * Pure EPUB spine + navigation assembly — the reading-order core of the reader.
 *
 * Given a parsed OPF (and, for the TOC, the EPUB3 `nav` document or the EPUB2
 * `toc.ncx`), these functions produce the ordered reading **spine**, the manifest
 * lookup, and a flat table of contents — all with the browser-native `DOMParser`
 * only, **no zip / no File / no network**. The lazy reader shell
 * ({@link ../reader/render.openEpub}) unzips the bytes and calls into here; the
 * logic itself stays unit-testable against sample XML strings, exactly like the
 * P4 OPF parsers.
 *
 * Mirrors the blueprint reader's spine/resource-resolution intent
 * (`src/EpubReader.tsx`), re-expressed as pure functions rather than an epub.js
 * rendition.
 */

import { resolveRelativePath } from './opf';

/** A manifest entry: a single resource declared by the OPF. */
export interface ManifestItem {
  id: string;
  /** Full in-zip path (resolved against the OPF's directory). */
  href: string;
  mediaType: string;
  /** `properties` tokens (e.g. `nav`, `cover-image`), split on whitespace. */
  properties: string[];
}

/** One document in reading order. */
export interface SpineItem {
  idref: string;
  /** Full in-zip path of the document. */
  href: string;
  mediaType: string;
}

/** Where the table of contents lives, and in which format. */
export interface TocRef {
  kind: 'nav' | 'ncx';
  /** Full in-zip path of the nav/ncx document. */
  href: string;
}

/** The parsed reading structure of an EPUB. */
export interface EpubStructure {
  spine: SpineItem[];
  manifest: Map<string, ManifestItem>;
  toc?: TocRef;
}

/** One entry in a flat table of contents. */
export interface TocEntry {
  title: string;
  /** Full in-zip path (may include a `#fragment`). */
  href: string;
}

/**
 * Assemble the manifest, reading-order spine, and TOC reference from an OPF.
 * Pure. Malformed XML yields an empty structure rather than throwing.
 */
export function assembleSpine(opfXml: string, opfPath: string): EpubStructure {
  const empty: EpubStructure = { spine: [], manifest: new Map() };

  const doc = new DOMParser().parseFromString(opfXml, 'application/xml');
  if (doc.getElementsByTagName('parsererror').length > 0) return empty;

  const manifest = new Map<string, ManifestItem>();
  const manifestEl = elementByLocalName(doc, 'manifest');
  if (manifestEl) {
    for (const item of childrenByLocalName(manifestEl, 'item')) {
      const id = item.getAttribute('id');
      const rawHref = item.getAttribute('href');
      if (!id || !rawHref) continue;
      manifest.set(id, {
        id,
        href: resolveRelativePath(opfPath, rawHref),
        mediaType: item.getAttribute('media-type') ?? '',
        properties: (item.getAttribute('properties') ?? '').split(/\s+/).filter(Boolean),
      });
    }
  }

  const spine: SpineItem[] = [];
  const spineEl = elementByLocalName(doc, 'spine');
  if (spineEl) {
    for (const ref of childrenByLocalName(spineEl, 'itemref')) {
      const idref = ref.getAttribute('idref');
      if (!idref) continue;
      const item = manifest.get(idref);
      if (!item) continue;
      spine.push({ idref, href: item.href, mediaType: item.mediaType });
    }
  }

  return { spine, manifest, toc: findToc(manifest, spineEl) };
}

/**
 * Parse an EPUB3 navigation document (`nav.xhtml`) into a flat TOC. Pure.
 *
 * Prefers the `<nav epub:type="toc">`; falls back to the first `<nav>`. Anchor
 * hrefs are resolved against the nav document's own directory.
 */
export function parseNavDoc(xhtml: string, navPath: string): TocEntry[] {
  const doc = new DOMParser().parseFromString(xhtml, 'text/html');
  const navs = Array.from(doc.getElementsByTagName('nav'));
  const nav = navs.find((n) => (n.getAttribute('epub:type') ?? '').includes('toc')) ?? navs[0];
  if (!nav) return [];

  const entries: TocEntry[] = [];
  for (const anchor of Array.from(nav.getElementsByTagName('a'))) {
    const rawHref = anchor.getAttribute('href');
    const title = anchor.textContent?.trim() ?? '';
    if (!rawHref || !title) continue;
    entries.push({ title, href: resolveRelativePath(navPath, rawHref) });
  }
  return entries;
}

/**
 * Parse an EPUB2 NCX (`toc.ncx`) into a flat TOC, in `navMap` document order
 * (nested `navPoint`s flattened). Pure. Hrefs are resolved against the NCX's dir.
 */
export function parseNcx(xml: string, ncxPath: string): TocEntry[] {
  const doc = new DOMParser().parseFromString(xml, 'application/xml');
  if (doc.getElementsByTagName('parsererror').length > 0) return [];

  const entries: TocEntry[] = [];
  for (const point of Array.from(doc.getElementsByTagNameNS('*', 'navPoint'))) {
    const title = firstDescendantText(point, 'text');
    const src = firstDescendant(point, 'content')?.getAttribute('src') ?? '';
    if (!title || !src) continue;
    entries.push({ title, href: resolveRelativePath(ncxPath, src) });
  }
  return entries;
}

/**
 * Find the spine index whose document matches `href`, ignoring any `#fragment`.
 * Returns `-1` when there is no match. Pure.
 */
export function spineIndexForHref(spine: readonly SpineItem[], href: string): number {
  const target = stripFragment(href);
  return spine.findIndex((item) => stripFragment(item.href) === target);
}

/** Locate the TOC: EPUB3 `nav` manifest property, else the EPUB2 spine `toc` id. */
function findToc(
  manifest: Map<string, ManifestItem>,
  spineEl: Element | undefined,
): TocRef | undefined {
  for (const item of manifest.values()) {
    if (item.properties.includes('nav')) return { kind: 'nav', href: item.href };
  }
  const ncxId = spineEl?.getAttribute('toc');
  const ncx = ncxId ? manifest.get(ncxId) : undefined;
  return ncx ? { kind: 'ncx', href: ncx.href } : undefined;
}

function stripFragment(href: string): string {
  const hash = href.indexOf('#');
  return hash >= 0 ? href.slice(0, hash) : href;
}

function elementByLocalName(doc: Document, localName: string): Element | undefined {
  return doc.getElementsByTagNameNS('*', localName)[0];
}

function childrenByLocalName(parent: Element, localName: string): Element[] {
  return Array.from(parent.children).filter((child) => child.localName === localName);
}

function firstDescendant(el: Element, localName: string): Element | undefined {
  return el.getElementsByTagNameNS('*', localName)[0];
}

function firstDescendantText(el: Element, localName: string): string {
  return firstDescendant(el, localName)?.textContent?.trim() ?? '';
}
