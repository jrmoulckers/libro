import { describe, expect, it } from 'vitest';
import type { Book } from '../models';
import { InMemoryLocalStore, isLocalCoverUrl, localCoverUrl, LOCALCOVER_SCHEME } from './store';

function book(id: string): Book {
  return {
    id,
    title: `Book ${id}`,
    authors: ['An Author'],
    mediaType: 'ebook',
    sourceProviderId: 'localfiles',
  };
}

describe('InMemoryLocalStore', () => {
  it('round-trips book metadata, file and cover by id', async () => {
    const store = new InMemoryLocalStore();
    const file = new Blob(['epub-bytes'], { type: 'application/epub+zip' });
    const cover = new Blob(['cover-bytes'], { type: 'image/png' });

    await store.put({ book: book('a'), file, cover });

    expect(await store.has('a')).toBe(true);
    expect(await store.getBook('a')).toMatchObject({ id: 'a', title: 'Book a' });
    expect(await store.getFile('a')).toBe(file);
    expect(await store.getCover('a')).toBe(cover);
  });

  it('lists every stored book and reports misses', async () => {
    const store = new InMemoryLocalStore();
    await store.put({ book: book('a'), file: new Blob(['a']) });
    await store.put({ book: book('b'), file: new Blob(['b']) });

    const ids = (await store.listBooks()).map((entry) => entry.id).sort();
    expect(ids).toEqual(['a', 'b']);
    expect(await store.has('missing')).toBe(false);
    expect(await store.getBook('missing')).toBeUndefined();
    expect(await store.getCover('a')).toBeUndefined();
  });

  it('clones on the way out so stored state is immutable by reference', async () => {
    const store = new InMemoryLocalStore();
    await store.put({ book: book('a'), file: new Blob(['a']) });

    const fetched = await store.getBook('a');
    fetched!.title = 'mutated';
    expect((await store.getBook('a'))!.title).toBe('Book a');
  });

  it('clears everything', async () => {
    const store = new InMemoryLocalStore();
    await store.put({ book: book('a'), file: new Blob(['a']) });
    await store.clear();
    expect(await store.listBooks()).toEqual([]);
  });
});

describe('local cover URLs', () => {
  it('builds and recognizes the localcover scheme', () => {
    expect(localCoverUrl('abc')).toBe(`${LOCALCOVER_SCHEME}abc`);
    expect(isLocalCoverUrl(localCoverUrl('abc'))).toBe(true);
    expect(isLocalCoverUrl('https://example.com/c.jpg')).toBe(false);
    expect(isLocalCoverUrl(undefined)).toBe(false);
  });
});
