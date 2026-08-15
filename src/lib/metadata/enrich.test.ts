import { describe, expect, it, vi } from 'vitest';
import { enrichBooks, type EnrichDeps } from './enrich';
import { InMemoryMetadataCache } from './cache';
import { MetadataError, type MetadataPatch } from './types';
import type { Book } from '../models';

function book(id: string, isbn: string | null, overrides: Partial<Book> = {}): Book {
  return {
    id,
    title: `Book ${id}`,
    authors: [],
    mediaType: 'ebook',
    sourceProviderId: 'abs',
    identifiers: isbn ? { isbn_13: isbn } : {},
    ...overrides,
  };
}

/** A fake Open Library batch fetcher backed by a fixture map, with a call spy. */
function fakeOl(fixture: Record<string, MetadataPatch>) {
  const spy = vi.fn(async (isbns: string[]) => {
    const map = new Map<string, MetadataPatch>();
    for (const isbn of isbns) map.set(isbn, fixture[isbn] ?? {});
    return map;
  });
  return spy;
}

const ISBN_A = '9780134685991';
const ISBN_B = '9780596007126';

describe('enrichBooks', () => {
  it('fills only gaps and never clobbers connector data', async () => {
    const ol = fakeOl({ [ISBN_A]: { coverUrl: 'ol-cover', authors: ['Joshua Bloch'] } });
    const gb = vi.fn(async () => ({ description: 'gb-desc' }) as MetadataPatch);
    const cache = new InMemoryMetadataCache();
    const deps: EnrichDeps = { cache, fetchOpenLibrary: ol, fetchGoogleBooks: gb };

    const input = book('1', ISBN_A, { coverUrl: 'existing-cover' });
    const { books, errors } = await enrichBooks([input], deps);

    expect(errors).toEqual([]);
    expect(books[0]!.coverUrl).toBe('existing-cover'); // preserved
    expect(books[0]!.authors).toEqual(['Joshua Bloch']); // filled by OL
    expect(books[0]!.description).toBe('gb-desc'); // filled by GB fallback
    expect(ol).toHaveBeenCalledTimes(1);
    expect(gb).toHaveBeenCalledTimes(1); // description still missing after OL
    // The combined patch was cached.
    expect(await cache.get(ISBN_A)).toEqual({
      coverUrl: 'ol-cover',
      authors: ['Joshua Bloch'],
      description: 'gb-desc',
    });
  });

  it('de-duplicates identical ISBNs to a single lookup', async () => {
    const ol = fakeOl({
      [ISBN_A]: { authors: ['A'], description: 'd', coverUrl: 'c', series: 's', subjects: ['x'] },
    });
    const gb = vi.fn(async () => ({}) as MetadataPatch);
    const deps: EnrichDeps = {
      cache: new InMemoryMetadataCache(),
      fetchOpenLibrary: ol,
      fetchGoogleBooks: gb,
    };

    const { books } = await enrichBooks([book('1', ISBN_A), book('2', ISBN_A)], deps);

    expect(books).toHaveLength(2);
    expect(books.every((b) => b.description === 'd')).toBe(true);
    expect(ol).toHaveBeenCalledTimes(1);
    expect(ol).toHaveBeenCalledWith([ISBN_A]); // one unique ISBN
    expect(gb).not.toHaveBeenCalled(); // OL supplied every needed field
  });

  it('skips network entirely on a cache hit', async () => {
    const cache = new InMemoryMetadataCache();
    await cache.set(ISBN_A, { description: 'cached', coverUrl: 'c', series: 's', subjects: ['x'] });
    const ol = fakeOl({});
    const gb = vi.fn(async () => ({}) as MetadataPatch);

    const { books } = await enrichBooks([book('1', ISBN_A)], {
      cache,
      fetchOpenLibrary: ol,
      fetchGoogleBooks: gb,
    });

    expect(books[0]!.description).toBe('cached');
    expect(ol).not.toHaveBeenCalled();
    expect(gb).not.toHaveBeenCalled();
  });

  it('caches a negative result so it is never re-fetched', async () => {
    const cache = new InMemoryMetadataCache();
    const ol = fakeOl({}); // nothing found
    const gb = vi.fn(async () => ({}) as MetadataPatch); // nothing found
    const deps: EnrichDeps = { cache, fetchOpenLibrary: ol, fetchGoogleBooks: gb };

    const first = await enrichBooks([book('1', ISBN_A)], deps);
    expect(first.books[0]!.description).toBeUndefined(); // unchanged
    expect(await cache.get(ISBN_A)).toEqual({}); // empty patch cached

    // A second run reuses the cache — no further network.
    ol.mockClear();
    gb.mockClear();
    await enrichBooks([book('1', ISBN_A)], deps);
    expect(ol).not.toHaveBeenCalled();
    expect(gb).not.toHaveBeenCalled();
  });

  it('isolates a failing Google Books lookup without dropping books', async () => {
    const ol = fakeOl({ [ISBN_A]: { coverUrl: 'c' }, [ISBN_B]: { coverUrl: 'c2' } });
    const gb = vi.fn(async (isbn: string) => {
      if (isbn === ISBN_A) throw new MetadataError('boom', 'googlebooks');
      return { description: 'ok' } as MetadataPatch;
    });
    const { books, errors } = await enrichBooks([book('1', ISBN_A), book('2', ISBN_B)], {
      cache: new InMemoryMetadataCache(),
      fetchOpenLibrary: ol,
      fetchGoogleBooks: gb,
    });

    expect(books).toHaveLength(2);
    expect(books[1]!.description).toBe('ok'); // the other book still enriched
    expect(errors).toEqual([{ isbn: ISBN_A, source: 'googlebooks', reason: 'boom' }]);
  });

  it('tolerates an Open Library batch failure and still tries Google Books', async () => {
    const ol = vi.fn(async () => {
      throw new MetadataError('OL down', 'openlibrary');
    });
    const gb = vi.fn(async () => ({ description: 'from-gb', coverUrl: 'c' }) as MetadataPatch);
    const { books, errors } = await enrichBooks([book('1', ISBN_A)], {
      cache: new InMemoryMetadataCache(),
      fetchOpenLibrary: ol,
      fetchGoogleBooks: gb,
    });

    expect(errors[0]?.source).toBe('openlibrary');
    expect(books[0]!.description).toBe('from-gb'); // fallback carried the load
  });

  it('passes through books without an ISBN or without gaps, preserving order', async () => {
    const ol = fakeOl({
      [ISBN_A]: { description: 'd', coverUrl: 'c', series: 's', subjects: ['x'] },
    });
    const gb = vi.fn(async () => ({}) as MetadataPatch);
    const complete = book('complete', ISBN_B, {
      authors: ['A'],
      coverUrl: 'c',
      description: 'd',
      series: 's',
      subjects: ['x'],
    });
    const noIsbn = book('noisbn', null);

    const { books } = await enrichBooks([noIsbn, book('needs', ISBN_A), complete], {
      cache: new InMemoryMetadataCache(),
      fetchOpenLibrary: ol,
      fetchGoogleBooks: gb,
    });

    expect(books.map((b) => b.id)).toEqual(['noisbn', 'needs', 'complete']); // order kept
    expect(books[1]!.description).toBe('d');
    // Only the one eligible ISBN was requested.
    expect(ol).toHaveBeenCalledWith([ISBN_A]);
  });
});
