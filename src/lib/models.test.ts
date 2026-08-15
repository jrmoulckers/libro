import { describe, expect, it } from 'vitest';
import {
  bookKey,
  countByMediaType,
  dedupeBooks,
  groupByMediaType,
  sortBooks,
  type Book,
} from './models';

function book(partial: Partial<Book> & Pick<Book, 'id' | 'title'>): Book {
  return {
    authors: [],
    mediaType: 'ebook',
    sourceProviderId: 'test',
    ...partial,
  };
}

const library: Book[] = [
  book({ id: '2', title: 'Piranesi', authors: ['Clarke, Susanna'], mediaType: 'ebook' }),
  book({ id: '1', title: 'Babel', authors: ['Kuang, R. F.'], mediaType: 'audiobook' }),
  book({
    id: '3',
    title: 'Ancillary Justice',
    authors: ['Clarke, Susanna'],
    mediaType: 'audiobook',
  }),
];

describe('sortBooks', () => {
  it('orders by first author then title without mutating the input', () => {
    const sorted = sortBooks(library);

    expect(sorted.map((b) => b.id)).toEqual(['3', '2', '1']);
    expect(library.map((b) => b.id)).toEqual(['2', '1', '3']);
  });

  it('treats missing authors as empty', () => {
    const sorted = sortBooks([
      book({ id: 'b', title: 'B', authors: ['Author'] }),
      book({ id: 'a', title: 'A', authors: [] }),
    ]);

    expect(sorted.map((b) => b.id)).toEqual(['a', 'b']);
  });
});

describe('countByMediaType', () => {
  it('counts items of a single media type', () => {
    expect(countByMediaType(library, 'audiobook')).toBe(2);
    expect(countByMediaType(library, 'ebook')).toBe(1);
    expect(countByMediaType(library, 'podcast')).toBe(0);
  });
});

describe('groupByMediaType', () => {
  it('buckets every media type, empty groups included', () => {
    const groups = groupByMediaType(library);

    expect(groups.get('ebook')?.map((b) => b.id)).toEqual(['2']);
    expect(groups.get('audiobook')?.map((b) => b.id)).toEqual(['1', '3']);
    expect(groups.get('podcast')).toEqual([]);
  });
});

describe('bookKey', () => {
  it('combines source provider id and local id', () => {
    expect(bookKey({ id: '42', sourceProviderId: 'mock' })).toBe('mock:42');
  });
});

describe('dedupeBooks', () => {
  it('drops duplicates that share an identifier, keeping the first', () => {
    const merged = dedupeBooks([
      book({
        id: 'a1',
        title: 'Dune',
        sourceProviderId: 'abs',
        identifiers: { isbn: '978-0441013593' },
      }),
      book({
        id: 'b1',
        title: 'Dune (unabridged)',
        sourceProviderId: 'opds',
        identifiers: { ISBN: '978-0441013593' },
      }),
    ]);

    expect(merged).toHaveLength(1);
    expect(merged[0]!.sourceProviderId).toBe('abs');
  });

  it('falls back to a title/authors/mediaType signature when identifiers differ', () => {
    const merged = dedupeBooks([
      book({ id: 'a', title: 'Babel', authors: ['R. F. Kuang'], mediaType: 'ebook' }),
      book({ id: 'b', title: '  babel ', authors: ['r. f. kuang'], mediaType: 'ebook' }),
    ]);

    expect(merged).toHaveLength(1);
    expect(merged[0]!.id).toBe('a');
  });

  it('keeps items that only differ by media type', () => {
    const merged = dedupeBooks([
      book({ id: 'a', title: 'Babel', authors: ['Kuang'], mediaType: 'ebook' }),
      book({ id: 'b', title: 'Babel', authors: ['Kuang'], mediaType: 'audiobook' }),
    ]);

    expect(merged).toHaveLength(2);
  });

  it('preserves input order for distinct items', () => {
    const merged = dedupeBooks([
      book({ id: '1', title: 'Zed' }),
      book({ id: '2', title: 'Alpha' }),
    ]);

    expect(merged.map((b) => b.id)).toEqual(['1', '2']);
  });
});
