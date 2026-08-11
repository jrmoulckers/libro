import { describe, expect, it } from 'vitest';
import { AbsError, mapAbsItem, mapAbsLibraryItems } from './audiobookshelf';

const BASE_URL = 'https://abs.example.com';
const PROVIDER_ID = 'abs-home';
const TOKEN = 'TESTTOKEN';

// Representative minified `/api/libraries/{id}/items` payload, shaped after the
// public ABS API docs. Item 1: minified authorName CSV + series + cover + ids.
// Item 2: full metadata (authors[].name + series[].name), no cover. Item 3: no
// title (must be skipped).
const ITEMS_RESPONSE = {
  results: [
    {
      id: 'li_fellowship',
      mediaType: 'book',
      media: {
        metadata: {
          title: 'The Fellowship of the Ring',
          authorName: 'J. R. R. Tolkien, Someone Else',
          seriesName: 'The Lord of the Rings',
          description: 'The first volume.',
          isbn: '9780547928210',
          asin: 'B007978NPG',
        },
        coverPath: '/metadata/items/li_fellowship/cover.jpg',
        numTracks: 12,
      },
    },
    {
      id: 'li_fullmeta',
      media: {
        metadata: {
          title: 'Children of Time',
          authors: [{ name: 'Adrian Tchaikovsky' }],
          series: [{ name: 'Children of Time', sequence: '1' }],
        },
      },
    },
    {
      id: 'li_notitle',
      media: { metadata: { authorName: 'Nobody' } },
    },
  ],
  total: 3,
};

describe('mapAbsLibraryItems', () => {
  const books = mapAbsLibraryItems(ITEMS_RESPONSE, BASE_URL, PROVIDER_ID, TOKEN);

  it('skips items without a title', () => {
    expect(books).toHaveLength(2);
    expect(books.map((b) => b.title)).toEqual(['The Fellowship of the Ring', 'Children of Time']);
  });

  it('maps minified metadata: CSV authors, series, description, ids as audiobook', () => {
    const [fellowship] = books;
    expect(fellowship!.id).toBe('abs-li_fellowship');
    expect(fellowship!.mediaType).toBe('audiobook');
    expect(fellowship!.sourceProviderId).toBe(PROVIDER_ID);
    expect(fellowship!.authors).toEqual(['J. R. R. Tolkien', 'Someone Else']);
    expect(fellowship!.series).toBe('The Lord of the Rings');
    expect(fellowship!.description).toBe('The first volume.');
    expect(fellowship!.identifiers).toMatchObject({
      'abs:item_id': 'li_fellowship',
      isbn: '9780547928210',
      asin: 'B007978NPG',
    });
  });

  it('constructs a token-bearing cover URL when the item has a cover', () => {
    expect(books[0]!.coverUrl).toBe(
      'https://abs.example.com/api/items/li_fellowship/cover?token=TESTTOKEN',
    );
  });

  it('maps full (structured) authors[].name and series[].name', () => {
    const [, childrenOfTime] = books;
    expect(childrenOfTime!.authors).toEqual(['Adrian Tchaikovsky']);
    expect(childrenOfTime!.series).toBe('Children of Time');
    expect(childrenOfTime!.coverUrl).toBeUndefined();
    expect(childrenOfTime!.identifiers).toEqual({ 'abs:item_id': 'li_fullmeta' });
  });

  it('accepts a bare items array as well as the wrapped response', () => {
    const fromArray = mapAbsLibraryItems(ITEMS_RESPONSE.results, BASE_URL, PROVIDER_ID, TOKEN);
    expect(fromArray).toHaveLength(2);
  });

  it('returns an empty list for unexpected shapes', () => {
    expect(mapAbsLibraryItems(null, BASE_URL, PROVIDER_ID)).toEqual([]);
    expect(mapAbsLibraryItems({ results: 'nope' }, BASE_URL, PROVIDER_ID)).toEqual([]);
  });
});

describe('mapAbsItem', () => {
  it('omits the token from the cover URL when none is configured', () => {
    const book = mapAbsItem(
      { id: 'li_x', media: { metadata: { title: 'X' }, coverPath: '/c.jpg' } },
      `${BASE_URL}/`,
      PROVIDER_ID,
    );
    expect(book?.coverUrl).toBe('https://abs.example.com/api/items/li_x/cover');
  });

  it('returns null for items missing an id or title', () => {
    expect(
      mapAbsItem({ media: { metadata: { title: 'No id' } } }, BASE_URL, PROVIDER_ID),
    ).toBeNull();
    expect(mapAbsItem({ id: 'li_y', media: {} }, BASE_URL, PROVIDER_ID)).toBeNull();
  });
});

describe('AbsError', () => {
  it('is a named Error carrying an optional cause', () => {
    const cause = new Error('boom');
    const err = new AbsError('failed', cause);
    expect(err).toBeInstanceOf(Error);
    expect(err.name).toBe('AbsError');
    expect(err.cause).toBe(cause);
  });
});
