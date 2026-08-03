import { describe, expect, it } from 'vitest';
import type { Book } from '../models';
import { aggregateLibrary, ProviderRegistry } from './registry';
import type { Provider, ProviderCapability } from './types';

function makeProvider(
  id: string,
  listBooks: () => Promise<Book[]>,
  capabilities: ProviderCapability[] = ['catalog'],
): Provider {
  return {
    id,
    displayName: id,
    capabilities: new Set(capabilities),
    listBooks,
  };
}

function book(id: string, sourceProviderId: string, overrides: Partial<Book> = {}): Book {
  return {
    id,
    title: `Title ${id}`,
    authors: ['Author'],
    mediaType: 'ebook',
    sourceProviderId,
    ...overrides,
  };
}

describe('ProviderRegistry', () => {
  it('lists providers in registration order and dedupes by id', () => {
    const a = makeProvider('a', async () => []);
    const b = makeProvider('b', async () => []);
    const a2 = makeProvider('a', async () => []);

    const registry = new ProviderRegistry([a, b]);
    registry.register(a2);

    expect(registry.list()).toEqual([a2, b]);
  });

  it('unregisters providers', () => {
    const registry = new ProviderRegistry([makeProvider('a', async () => [])]);
    expect(registry.unregister('a')).toBe(true);
    expect(registry.unregister('a')).toBe(false);
    expect(registry.list()).toEqual([]);
  });
});

describe('aggregateLibrary', () => {
  it('merges books from every provider', async () => {
    const result = await aggregateLibrary([
      makeProvider('p1', async () => [book('1', 'p1')]),
      makeProvider('p2', async () => [book('2', 'p2')]),
    ]);

    expect(result.errors).toEqual([]);
    expect(result.books.map((b) => b.sourceProviderId).sort()).toEqual(['p1', 'p2']);
  });

  it('isolates a throwing provider without aborting the aggregate', async () => {
    const boom = new Error('network down');
    const result = await aggregateLibrary([
      makeProvider('good', async () => [book('1', 'good')]),
      makeProvider('bad', async () => {
        throw boom;
      }),
    ]);

    expect(result.books).toHaveLength(1);
    expect(result.books[0].sourceProviderId).toBe('good');
    expect(result.errors).toEqual([{ providerId: 'bad', error: boom }]);
  });

  it('dedupes the same title arriving from two providers', async () => {
    const result = await aggregateLibrary([
      makeProvider('abs', async () => [book('a', 'abs', { identifiers: { isbn: '111' } })]),
      makeProvider('opds', async () => [book('b', 'opds', { identifiers: { isbn: '111' } })]),
    ]);

    expect(result.books).toHaveLength(1);
    expect(result.books[0].sourceProviderId).toBe('abs');
  });

  it('accepts a registry as its source', async () => {
    const registry = new ProviderRegistry([makeProvider('p1', async () => [book('1', 'p1')])]);
    const result = await aggregateLibrary(registry);

    expect(result.books).toHaveLength(1);
  });

  it('returns an empty catalog for no providers', async () => {
    const result = await aggregateLibrary([]);
    expect(result).toEqual({ books: [], errors: [] });
  });
});
