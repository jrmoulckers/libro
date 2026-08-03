import { describe, expect, it } from 'vitest';
import type { Book } from '../models';
import { InMemoryLibraryIndex } from './memory';
import type { LibraryIndex } from './types';

function book(id: string, sourceProviderId = 'mock', overrides: Partial<Book> = {}): Book {
  return {
    id,
    title: `Title ${id}`,
    authors: ['Author'],
    mediaType: 'ebook',
    sourceProviderId,
    ...overrides,
  };
}

/**
 * Contract tests, written against the {@link LibraryIndex} interface so they can be
 * reused for any implementation. Run here against the in-memory fake.
 */
function runContract(name: string, create: () => LibraryIndex): void {
  describe(name, () => {
    it('stores and retrieves a book by its bookKey', async () => {
      const index = create();
      await index.put([book('1')]);

      expect(await index.get('mock:1')).toMatchObject({ id: '1' });
      expect(await index.get('missing')).toBeUndefined();
    });

    it('lists every stored book', async () => {
      const index = create();
      await index.put([book('1'), book('2')]);

      const ids = (await index.list()).map((b) => b.id).sort();
      expect(ids).toEqual(['1', '2']);
    });

    it('upserts on matching key and keeps different providers apart', async () => {
      const index = create();
      await index.put([book('1', 'a', { title: 'First' })]);
      await index.put([book('1', 'a', { title: 'Updated' })]);
      await index.put([book('1', 'b', { title: 'Other provider' })]);

      expect((await index.get('a:1'))?.title).toBe('Updated');
      expect((await index.get('b:1'))?.title).toBe('Other provider');
      expect(await index.list()).toHaveLength(2);
    });

    it('clears the index', async () => {
      const index = create();
      await index.put([book('1')]);
      await index.clear();

      expect(await index.list()).toEqual([]);
    });

    it('does not alias stored objects with caller-held references', async () => {
      const index = create();
      const original = book('1', 'mock', { authors: ['A'] });
      await index.put([original]);
      original.authors.push('Injected');

      expect((await index.get('mock:1'))?.authors).toEqual(['A']);
    });
  });
}

runContract('InMemoryLibraryIndex', () => new InMemoryLibraryIndex());
