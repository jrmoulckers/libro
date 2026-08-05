import { describe, expect, it } from 'vitest';
import { googleBooksUrl, parseGoogleBooks } from './googlebooks';

// Shaped after a live `?q=isbn:9780134685991` response (volumeInfo trimmed).
const VOLUMES = {
  kind: 'books#volumes',
  totalItems: 1,
  items: [
    {
      id: 'BIpDDwAAQBAJ',
      volumeInfo: {
        title: 'Effective Java',
        authors: ['Joshua Bloch'],
        description: 'The definitive guide to Java best practices.',
        categories: ['Computers / Programming Languages / Java'],
        imageLinks: {
          smallThumbnail: 'http://books.google.com/books/content?id=BIpDDwAAQBAJ&img=1&zoom=5',
          thumbnail: 'http://books.google.com/books/content?id=BIpDDwAAQBAJ&img=1&zoom=1',
        },
      },
    },
  ],
};

describe('googleBooksUrl', () => {
  it('builds an isbn query URL', () => {
    const url = googleBooksUrl('9780134685991');
    expect(url).toContain('/books/v1/volumes?q=');
    expect(url).toContain(encodeURIComponent('isbn:9780134685991'));
  });
});

describe('parseGoogleBooks', () => {
  it('extracts description, authors, categories, and upgrades the cover to https', () => {
    const patch = parseGoogleBooks(VOLUMES);
    expect(patch.description).toBe('The definitive guide to Java best practices.');
    expect(patch.authors).toEqual(['Joshua Bloch']);
    expect(patch.subjects).toEqual(['Computers / Programming Languages / Java']);
    expect(patch.coverUrl?.startsWith('https://')).toBe(true); // thumbnail preferred + https
    expect(patch.coverUrl).toContain('zoom=1');
    expect(patch.series).toBeUndefined(); // Google Books has no series concept
  });

  it('returns an empty patch when there are no items', () => {
    expect(parseGoogleBooks({ kind: 'books#volumes', totalItems: 0 })).toEqual({});
    expect(parseGoogleBooks({})).toEqual({});
    expect(parseGoogleBooks(null)).toEqual({});
  });

  it('tolerates a volume missing imageLinks', () => {
    const patch = parseGoogleBooks({ items: [{ volumeInfo: { description: 'd' } }] });
    expect(patch).toEqual({ description: 'd' });
  });
});
