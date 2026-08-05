import { describe, expect, it } from 'vitest';
import { assembleSpine, parseNavDoc, parseNcx, spineIndexForHref } from './spine';

const OPF_NAV = `<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Nav Book</dc:title></metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="c1" href="text/chap1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/chap2.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="styles/main.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>`;

const OPF_NCX = `<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Ncx Book</dc:title></metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="c1" href="chap1.xhtml" media-type="application/xhtml+xml"/>
    <item id="missing-ref" href="orphan.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="c1"/>
    <itemref idref="nope"/>
  </spine>
</package>`;

const NAV_XHTML = `<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="landmarks"><ol><li><a href="text/chap1.xhtml">Start</a></li></ol></nav>
    <nav epub:type="toc">
      <ol>
        <li><a href="text/chap1.xhtml">Chapter One</a>
          <ol><li><a href="text/chap2.xhtml#s2">Section Two</a></li></ol>
        </li>
      </ol>
    </nav>
  </body>
</html>`;

const NCX_XML = `<?xml version="1.0" encoding="utf-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <navMap>
    <navPoint id="np1"><navLabel><text>Chapter One</text></navLabel><content src="chap1.xhtml"/>
      <navPoint id="np1a"><navLabel><text>Part A</text></navLabel><content src="chap1.xhtml#a"/></navPoint>
    </navPoint>
  </navMap>
</ncx>`;

describe('assembleSpine', () => {
  it('builds the manifest map with resolved in-zip paths', () => {
    const { manifest } = assembleSpine(OPF_NAV, 'OEBPS/content.opf');
    expect(manifest.get('c1')?.href).toBe('OEBPS/text/chap1.xhtml');
    expect(manifest.get('css')?.mediaType).toBe('text/css');
    expect(manifest.get('nav')?.properties).toContain('nav');
  });

  it('builds the spine in itemref order with resolved paths', () => {
    const { spine } = assembleSpine(OPF_NAV, 'OEBPS/content.opf');
    expect(spine.map((s) => s.href)).toEqual(['OEBPS/text/chap1.xhtml', 'OEBPS/text/chap2.xhtml']);
  });

  it('detects the EPUB3 nav TOC via the manifest property', () => {
    const { toc } = assembleSpine(OPF_NAV, 'OEBPS/content.opf');
    expect(toc).toEqual({ kind: 'nav', href: 'OEBPS/nav.xhtml' });
  });

  it('detects the EPUB2 ncx TOC via the spine toc attribute and skips unknown idrefs', () => {
    const { spine, toc } = assembleSpine(OPF_NCX, 'toc.opf');
    expect(spine.map((s) => s.idref)).toEqual(['c1']); // "nope" has no manifest item
    expect(toc).toEqual({ kind: 'ncx', href: 'toc.ncx' });
  });

  it('returns an empty structure for malformed XML', () => {
    const { spine, manifest, toc } = assembleSpine('<package><broken', 'content.opf');
    expect(spine).toEqual([]);
    expect(manifest.size).toBe(0);
    expect(toc).toBeUndefined();
  });
});

describe('parseNavDoc', () => {
  it('reads the toc nav (not landmarks) and resolves hrefs', () => {
    const entries = parseNavDoc(NAV_XHTML, 'OEBPS/nav.xhtml');
    expect(entries).toEqual([
      { title: 'Chapter One', href: 'OEBPS/text/chap1.xhtml' },
      { title: 'Section Two', href: 'OEBPS/text/chap2.xhtml#s2' },
    ]);
  });
});

describe('parseNcx', () => {
  it('flattens navPoints in document order and resolves hrefs', () => {
    const entries = parseNcx(NCX_XML, 'OEBPS/toc.ncx');
    expect(entries).toEqual([
      { title: 'Chapter One', href: 'OEBPS/chap1.xhtml' },
      { title: 'Part A', href: 'OEBPS/chap1.xhtml#a' },
    ]);
  });
});

describe('spineIndexForHref', () => {
  const { spine } = assembleSpine(OPF_NAV, 'OEBPS/content.opf');

  it('matches ignoring the fragment', () => {
    expect(spineIndexForHref(spine, 'OEBPS/text/chap2.xhtml#s2')).toBe(1);
    expect(spineIndexForHref(spine, 'OEBPS/text/chap1.xhtml')).toBe(0);
  });

  it('returns -1 when nothing matches', () => {
    expect(spineIndexForHref(spine, 'OEBPS/missing.xhtml')).toBe(-1);
  });
});
