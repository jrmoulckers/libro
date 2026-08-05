import { describe, expect, it } from 'vitest';
import { rewriteChapterHtml, type ResourceResolver } from './rewrite';

// A resolver that maps known in-zip paths to fake object URLs.
const resolver: ResourceResolver = (path) =>
  ({
    'OEBPS/images/pic.png': 'blob:pic',
    'OEBPS/styles/main.css': 'blob:css',
    'OEBPS/images/fig.svg': 'blob:fig',
  })[path];

const CHAPTER = 'OEBPS/text/chap1.xhtml';

describe('rewriteChapterHtml', () => {
  it('rewrites relative image and stylesheet refs to resolver URLs', () => {
    const html = `<html><head>
      <link rel="stylesheet" href="../styles/main.css"/>
      </head><body>
      <img src="../images/pic.png" alt="A picture"/>
      </body></html>`;
    const out = rewriteChapterHtml(html, CHAPTER, resolver);
    expect(out).toContain('href="blob:css"');
    expect(out).toContain('src="blob:pic"');
    expect(out).toContain('alt="A picture"');
  });

  it('preserves a fragment when rewriting', () => {
    const html = `<html><body><svg><image href="../images/fig.svg#frame"></image></svg></body></html>`;
    const out = rewriteChapterHtml(html, CHAPTER, resolver);
    expect(out).toContain('blob:fig#frame');
  });

  it('leaves absolute, data, and unknown refs untouched', () => {
    const html = `<html><body>
      <img src="https://cdn.example.com/x.png"/>
      <img src="data:image/png;base64,AAAA"/>
      <img src="../images/missing.png"/>
      </body></html>`;
    const out = rewriteChapterHtml(html, CHAPTER, resolver);
    expect(out).toContain('https://cdn.example.com/x.png');
    expect(out).toContain('data:image/png;base64,AAAA');
    expect(out).toContain('../images/missing.png');
  });

  it('strips script elements', () => {
    const html = `<html><body><p>Hi</p><script>alert(1)</script></body></html>`;
    const out = rewriteChapterHtml(html, CHAPTER, resolver);
    expect(out).not.toContain('alert(1)');
    expect(out).toContain('<p>Hi</p>');
  });
});
