import { describe, expect, it } from 'vitest';
import { basicAuthHeader, OpdsError, parseOpdsFeed } from './opds';

const CATALOG_URL = 'https://books.example.com/opds/books?page=1';
const PROVIDER_ID = 'opds-example';

// Realistic acquisition feed: entry 1 has a cover + acquisition + two authors,
// entry 2 uses a relative acquisition href + a thumbnail + series, entry 3 is a
// navigation entry with no acquisition link (must be skipped).
const FEED = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:dcterms="http://purl.org/dc/terms/">
  <title>All Books</title>
  <link rel="self" href="/opds/books?page=1" type="application/atom+xml;profile=opds-catalog"/>
  <link rel="next" href="/opds/books?page=2" type="application/atom+xml;profile=opds-catalog"/>
  <entry>
    <title>Effective Java</title>
    <id>urn:uuid:1111</id>
    <author><name>Joshua Bloch</name></author>
    <author><name>Neal Gafter</name></author>
    <summary>The definitive guide.</summary>
    <dcterms:identifier>urn:isbn:9780134685991</dcterms:identifier>
    <link rel="http://opds-spec.org/image" href="https://cdn.example.com/covers/1.jpg" type="image/jpeg"/>
    <link rel="http://opds-spec.org/acquisition" href="https://files.example.com/1.epub" type="application/epub+zip"/>
  </entry>
  <entry>
    <title>The Odyssey</title>
    <id>urn:uuid:2222</id>
    <author><name>Homer</name></author>
    <content>Epic poem.</content>
    <series>Classics</series>
    <link rel="http://opds-spec.org/image/thumbnail" href="../covers/2-thumb.jpg" type="image/jpeg"/>
    <link rel="http://opds-spec.org/acquisition/open-access" href="../files/2.epub" type="application/epub+zip"/>
  </entry>
  <entry>
    <title>By Author</title>
    <id>urn:uuid:nav</id>
    <link rel="subsection" href="/opds/authors" type="application/atom+xml;profile=opds-catalog;kind=navigation"/>
  </entry>
</feed>`;

describe('parseOpdsFeed', () => {
  const books = parseOpdsFeed(FEED, CATALOG_URL, PROVIDER_ID);

  it('skips entries without an acquisition link', () => {
    expect(books).toHaveLength(2);
    expect(books.map((b) => b.title)).toEqual(['Effective Java', 'The Odyssey']);
  });

  it('maps title, multiple authors, description, and cover', () => {
    const [effectiveJava] = books;
    expect(effectiveJava.authors).toEqual(['Joshua Bloch', 'Neal Gafter']);
    expect(effectiveJava.description).toBe('The definitive guide.');
    expect(effectiveJava.coverUrl).toBe('https://cdn.example.com/covers/1.jpg');
    expect(effectiveJava.mediaType).toBe('ebook');
    expect(effectiveJava.sourceProviderId).toBe(PROVIDER_ID);
  });

  it('stores the absolute acquisition URL in identifiers', () => {
    expect(books[0].identifiers?.['opds:acquisition_url']).toBe('https://files.example.com/1.epub');
    expect(books[0].identifiers?.['opds:acquisition_type']).toBe('application/epub+zip');
  });

  it('resolves relative acquisition and cover links against the catalog URL', () => {
    const [, odyssey] = books;
    expect(odyssey.identifiers?.['opds:acquisition_url']).toBe(
      'https://books.example.com/files/2.epub',
    );
    expect(odyssey.coverUrl).toBe('https://books.example.com/covers/2-thumb.jpg');
    expect(odyssey.series).toBe('Classics');
    expect(odyssey.description).toBe('Epic poem.');
  });

  it('derives a stable id from the acquisition URL', () => {
    expect(books[0].id).toMatch(/^opds-[0-9a-z]+$/);
    // Deterministic across runs on the same input.
    expect(parseOpdsFeed(FEED, CATALOG_URL, PROVIDER_ID)[0].id).toBe(books[0].id);
  });

  it('throws a typed OpdsError on malformed XML', () => {
    expect(() => parseOpdsFeed('<feed><entry>', CATALOG_URL, PROVIDER_ID)).toThrow(OpdsError);
  });

  it('returns an empty list for a feed with no entries', () => {
    const empty = `<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><title>Empty</title></feed>`;
    expect(parseOpdsFeed(empty, CATALOG_URL, PROVIDER_ID)).toEqual([]);
  });
});

describe('basicAuthHeader', () => {
  it('base64-encodes username:password', () => {
    expect(basicAuthHeader({ username: 'ada', password: 'lovelace' })).toBe(
      `Basic ${btoa('ada:lovelace')}`,
    );
  });
});
