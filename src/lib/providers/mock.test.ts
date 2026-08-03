import { describe, expect, it } from 'vitest';
import { createMockProvider, MOCK_PROVIDER_ID } from './mock';

describe('createMockProvider', () => {
  it('advertises only the catalog capability', () => {
    const provider = createMockProvider();
    expect(provider.id).toBe(MOCK_PROVIDER_ID);
    expect([...provider.capabilities]).toEqual(['catalog']);
  });

  it('stamps every book with the provider id as its source', async () => {
    const books = await createMockProvider().listBooks();

    expect(books.length).toBeGreaterThan(0);
    expect(books.every((b) => b.sourceProviderId === MOCK_PROVIDER_ID)).toBe(true);
  });

  it('returns books across multiple media types', async () => {
    const books = await createMockProvider().listBooks();
    const types = new Set(books.map((b) => b.mediaType));

    expect(types.has('ebook')).toBe(true);
    expect(types.has('audiobook')).toBe(true);
    expect(types.has('podcast')).toBe(true);
  });

  it('does not share author arrays between calls (no fixture mutation)', async () => {
    const provider = createMockProvider();
    const first = await provider.listBooks();
    first[0].authors.push('Injected');

    const second = await provider.listBooks();
    expect(second[0].authors).not.toContain('Injected');
  });

  it('accepts custom fixtures', async () => {
    const provider = createMockProvider([
      { id: 'x', title: 'X', authors: ['A'], mediaType: 'ebook' },
    ]);
    const books = await provider.listBooks();

    expect(books).toHaveLength(1);
    expect(books[0]).toMatchObject({ id: 'x', sourceProviderId: MOCK_PROVIDER_ID });
  });
});
