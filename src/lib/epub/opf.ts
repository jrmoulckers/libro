/**
 * Pure EPUB OPF / container parsers — the network-free, zip-free core of the
 * local-file import path.
 *
 * An EPUB is a ZIP whose `META-INF/container.xml` points at an OPF *package
 * document*; the OPF carries the Dublin Core metadata Libro cares about. These
 * two functions parse those XML documents with the browser-native `DOMParser`
 * only — **no zip, no `File`, no network** — so they can be unit-tested directly
 * against sample XML strings. {@link ../epub/epub.parseEpubFile} does the zip I/O
 * and calls into here.
 *
 * This mirrors the Rust blueprint's `localfiles.rs` OPF handling (Dublin Core
 * metadata, the Calibre `series` `<meta>`, and the `cover-image` manifest
 * property) re-expressed for the browser DOM.
 */

/** The Dublin Core elements namespace used by OPF metadata. */
const DC_NS = 'http://purl.org/dc/elements/1.1/';

/** Parsed subset of an OPF package document. */
export interface ParsedOpf {
  /** `dc:title` (trimmed); empty string when absent. */
  title: string;
  /** `dc:creator` values, in document order. */
  authors: string[];
  /** `dc:description`, when present and non-empty. */
  description?: string;
  /**
   * Identifier scheme -> value. ISBNs are normalized into `isbn13`/`isbn10`;
   * other `dc:identifier`s are keyed by their `opf:scheme` (lower-cased) or
   * `identifier` as a fallback.
   */
  identifiers: Record<string, string>;
  /** Series name from `<meta name="calibre:series">`, when present. */
  series?: string;
  /**
   * Cover image path **resolved into a full in-zip path** (relative to the OPF's
   * own directory), ready to look up in the unzipped entry map. Present only when
   * the manifest declares a cover.
   */
  coverHref?: string;
}

/**
 * Find the OPF rootfile path declared in `META-INF/container.xml`. Pure.
 *
 * @returns the `full-path` of the first `<rootfile>`, or `undefined` when the
 * document is malformed or declares no rootfile.
 */
export function parseContainerXml(xml: string): string | undefined {
  const doc = new DOMParser().parseFromString(xml, 'application/xml');
  if (doc.getElementsByTagName('parsererror').length > 0) return undefined;

  const rootfile = doc.getElementsByTagNameNS('*', 'rootfile')[0];
  const path = rootfile?.getAttribute('full-path')?.trim();
  return path ? path : undefined;
}

/**
 * Parse an OPF package document into a {@link ParsedOpf}. Pure: DOMParser only.
 *
 * `opfPath` is the OPF's own path inside the zip; it is used to resolve the
 * (relative) cover href into a full in-zip path. Malformed XML yields an empty
 * result rather than throwing — a book with no readable metadata still imports
 * with a filename-derived title (see {@link ../local/import.buildLocalBook}).
 */
export function parseOpf(xml: string, opfPath: string): ParsedOpf {
  const parsed: ParsedOpf = { title: '', authors: [], identifiers: {} };

  const doc = new DOMParser().parseFromString(xml, 'application/xml');
  if (doc.getElementsByTagName('parsererror').length > 0) return parsed;

  // Dublin Core metadata (namespace-qualified).
  parsed.title = dcText(doc, 'title');
  parsed.authors = dcTexts(doc, 'creator');
  const description = dcText(doc, 'description');
  if (description) parsed.description = description;

  for (const node of dcNodes(doc, 'identifier')) {
    classifyIdentifier(node, parsed.identifiers);
  }

  // Calibre series + the legacy cover pointer live in <meta> elements.
  let legacyCoverId: string | undefined;
  for (const meta of Array.from(doc.getElementsByTagName('meta'))) {
    const name = meta.getAttribute('name');
    if (name === 'calibre:series') {
      const content = meta.getAttribute('content')?.trim();
      if (content) parsed.series = content;
    } else if (name === 'cover') {
      legacyCoverId = meta.getAttribute('content') ?? undefined;
    }
  }

  const coverHref = resolveCoverHref(doc, legacyCoverId);
  if (coverHref) parsed.coverHref = resolveRelativePath(opfPath, coverHref);

  return parsed;
}

/**
 * Join a manifest href (relative to the OPF's directory) into a full in-zip
 * path, collapsing `.` and `..` segments. Pure.
 */
export function resolveRelativePath(opfPath: string, href: string): string {
  const base = opfPath.includes('/') ? opfPath.slice(0, opfPath.lastIndexOf('/')) : '';
  const segments: string[] = [];
  for (const segment of [...base.split('/'), ...href.split('/')]) {
    if (segment === '' || segment === '.') continue;
    if (segment === '..') segments.pop();
    else segments.push(segment);
  }
  return segments.join('/');
}

/** First `dc:<name>` element's trimmed text, or empty string. */
function dcText(doc: Document, name: string): string {
  return dcNodes(doc, name)[0]?.textContent?.trim() ?? '';
}

/** All non-empty trimmed texts of `dc:<name>` elements, in document order. */
function dcTexts(doc: Document, name: string): string[] {
  return dcNodes(doc, name)
    .map((node) => node.textContent?.trim() ?? '')
    .filter((text) => text.length > 0);
}

function dcNodes(doc: Document, name: string): Element[] {
  return Array.from(doc.getElementsByTagNameNS(DC_NS, name));
}

/** Sort a `dc:identifier` into an ISBN slot or a scheme-keyed bag. */
function classifyIdentifier(node: Element, into: Record<string, string>): void {
  const raw = node.textContent?.trim() ?? '';
  if (!raw) return;

  // `opf:scheme` is namespaced; match by local attribute name for robustness.
  const scheme = attributeByLocalName(node, 'scheme');
  const declaredIsbn = scheme?.toLowerCase() === 'isbn' || raw.toLowerCase().includes('isbn');

  const digits = extractIsbnDigits(raw);
  if ((declaredIsbn || isIsbnShaped(digits)) && digits) {
    if (digits.length === 13) {
      into.isbn13 ??= digits;
      return;
    }
    if (digits.length === 10) {
      into.isbn10 ??= digits;
      return;
    }
  }

  const key = scheme ? scheme.toLowerCase() : 'identifier';
  into[key] ??= raw;
}

/** Strip `urn:isbn:` and separators, upper-casing the ISBN-10 check char `X`. */
function extractIsbnDigits(raw: string): string {
  const lower = raw.toLowerCase();
  const body = lower.startsWith('urn:isbn:') ? lower.slice('urn:isbn:'.length) : lower;
  return Array.from(body)
    .filter((c) => (c >= '0' && c <= '9') || c === 'x')
    .map((c) => c.toUpperCase())
    .join('');
}

function isIsbnShaped(digits: string): boolean {
  return (digits.length === 13 && /^\d+$/.test(digits)) || digits.length === 10;
}

/**
 * Resolve the cover href from the manifest: prefer the EPUB3
 * `properties="cover-image"` item, else the legacy `<meta name="cover">` id.
 */
function resolveCoverHref(doc: Document, legacyId: string | undefined): string | undefined {
  const items = Array.from(doc.getElementsByTagNameNS('*', 'item'));

  const byProperty = items.find((item) =>
    (item.getAttribute('properties') ?? '').split(/\s+/).includes('cover-image'),
  );
  if (byProperty) return byProperty.getAttribute('href') ?? undefined;

  if (legacyId) {
    const byId = items.find((item) => item.getAttribute('id') === legacyId);
    if (byId) return byId.getAttribute('href') ?? undefined;
  }
  return undefined;
}

/** Read an attribute by its local name, ignoring any namespace prefix. */
function attributeByLocalName(node: Element, localName: string): string | undefined {
  for (const attr of Array.from(node.attributes)) {
    if (attr.localName === localName) return attr.value;
  }
  return undefined;
}
