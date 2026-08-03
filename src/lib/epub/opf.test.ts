import { describe, expect, it } from 'vitest';
import { parseContainerXml, parseOpf, resolveRelativePath } from './opf';

const CONTAINER_XML = `<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>`;

// A realistic EPUB3 OPF: two creators, a Calibre series, an ISBN identifier and a
// non-ISBN one, and an EPUB3 cover-image manifest item in a nested images/ dir.
const OPF_FULL = `<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Effective Java</dc:title>
    <dc:creator>Joshua Bloch</dc:creator>
    <dc:creator>Second Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:description>A description straight from the EPUB.</dc:description>
    <dc:identifier opf:scheme="ISBN">urn:isbn:978-0-13-468599-1</dc:identifier>
    <dc:identifier opf:scheme="calibre">abc-123</dc:identifier>
    <meta name="calibre:series" content="The Java Series"/>
    <meta name="cover" content="legacy-cover"/>
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover-img" href="images/cover.png" media-type="image/png" properties="cover-image"/>
    <item id="legacy-cover" href="images/legacy.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="content"/></spine>
</package>`;

// Legacy EPUB2-style: no properties="cover-image"; cover comes from <meta name="cover">.
const OPF_LEGACY_COVER = `<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>The Odyssey</dc:title>
    <dc:creator>Homer</dc:creator>
    <meta name="cover" content="cover"/>
  </metadata>
  <manifest>
    <item id="cover" href="cover.jpeg" media-type="image/jpeg"/>
  </manifest>
</package>`;

describe('parseContainerXml', () => {
  it('returns the OPF rootfile full-path', () => {
    expect(parseContainerXml(CONTAINER_XML)).toBe('OEBPS/content.opf');
  });

  it('returns undefined for malformed XML', () => {
    expect(parseContainerXml('<container><oops></container>')).toBeUndefined();
  });

  it('returns undefined when there is no rootfile', () => {
    expect(parseContainerXml('<container xmlns="urn:x"></container>')).toBeUndefined();
  });
});

describe('parseOpf', () => {
  it('maps title, authors, description and series', () => {
    const parsed = parseOpf(OPF_FULL, 'OEBPS/content.opf');
    expect(parsed.title).toBe('Effective Java');
    expect(parsed.authors).toEqual(['Joshua Bloch', 'Second Author']);
    expect(parsed.description).toBe('A description straight from the EPUB.');
    expect(parsed.series).toBe('The Java Series');
  });

  it('normalizes an ISBN identifier and keeps other schemes', () => {
    const parsed = parseOpf(OPF_FULL, 'OEBPS/content.opf');
    expect(parsed.identifiers.isbn13).toBe('9780134685991');
    expect(parsed.identifiers.calibre).toBe('abc-123');
  });

  it('prefers the EPUB3 cover-image item and resolves it against the OPF dir', () => {
    const parsed = parseOpf(OPF_FULL, 'OEBPS/content.opf');
    expect(parsed.coverHref).toBe('OEBPS/images/cover.png');
  });

  it('falls back to the legacy <meta name="cover"> pointer', () => {
    const parsed = parseOpf(OPF_LEGACY_COVER, 'content.opf');
    expect(parsed.coverHref).toBe('cover.jpeg');
  });

  it('returns an empty result for malformed XML rather than throwing', () => {
    const parsed = parseOpf('<package><broken', 'content.opf');
    expect(parsed.title).toBe('');
    expect(parsed.authors).toEqual([]);
    expect(parsed.coverHref).toBeUndefined();
  });
});

describe('resolveRelativePath', () => {
  it('joins a href against the OPF directory', () => {
    expect(resolveRelativePath('OEBPS/content.opf', 'images/cover.png')).toBe(
      'OEBPS/images/cover.png',
    );
  });

  it('collapses .. segments', () => {
    expect(resolveRelativePath('OEBPS/sub/content.opf', '../images/cover.png')).toBe(
      'OEBPS/images/cover.png',
    );
  });

  it('handles an OPF at the zip root', () => {
    expect(resolveRelativePath('content.opf', 'cover.png')).toBe('cover.png');
  });
});
