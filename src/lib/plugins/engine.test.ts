import { describe, it, expect } from 'vitest';
import {
  checkDomainAllowed,
  mapDeclarativeCatalog,
  createDeclarativePluginProvider,
  extractAuthors,
  joinUrl,
} from './engine';
import { validatePluginManifest, PluginError, type DeclarativePluginManifest } from './manifest';

function manifest(overrides: Record<string, unknown> = {}): DeclarativePluginManifest {
  return validatePluginManifest({
    id: 'example-rest-catalog',
    displayName: 'Example REST Catalog',
    kind: 'declarative',
    baseUrl: 'https://api.example-books.test',
    allowedDomains: ['api.example-books.test'],
    capabilities: ['catalog'],
    catalog: {
      endpoint: '/api/books',
      itemsPath: 'results',
      mediaType: 'ebook',
      fields: {
        id: 'id',
        title: 'title',
        authors: 'authors',
        series: 'series.name',
        cover: 'cover',
        description: 'summary',
        identifiers: 'identifiers',
      },
    },
    ...overrides,
  }) as DeclarativePluginManifest;
}

describe('checkDomainAllowed', () => {
  const allowed = ['api.example-books.test'];

  it('permits the exact host', () => {
    expect(checkDomainAllowed('https://api.example-books.test/api/books', allowed)).toBe(true);
  });

  it('permits a subdomain of an allowed host', () => {
    expect(checkDomainAllowed('https://cdn.api.example-books.test/x', allowed)).toBe(true);
  });

  it('denies a lookalike suffix-spoof host', () => {
    expect(checkDomainAllowed('https://api.example-books.test.evil.com/x', allowed)).toBe(false);
  });

  it('denies a host not on the allowlist', () => {
    expect(checkDomainAllowed('https://evil.example.com', allowed)).toBe(false);
  });

  it('is case-insensitive on the host', () => {
    expect(checkDomainAllowed('https://API.Example-Books.test/x', allowed)).toBe(true);
  });

  it('ignores the port', () => {
    expect(checkDomainAllowed('https://api.example-books.test:8443/x', allowed)).toBe(true);
  });

  it('drops userinfo before matching', () => {
    expect(checkDomainAllowed('https://user:pass@api.example-books.test/x', allowed)).toBe(true);
    expect(checkDomainAllowed('https://api.example-books.test@evil.com/x', allowed)).toBe(false);
  });

  it('requires an http(s) scheme', () => {
    expect(checkDomainAllowed('ftp://api.example-books.test/x', allowed)).toBe(false);
    expect(checkDomainAllowed('api.example-books.test/x', allowed)).toBe(false);
  });
});

describe('mapDeclarativeCatalog', () => {
  it('maps items to books and skips malformed ones', () => {
    const json = {
      results: [
        {
          id: 'b1',
          title: 'Sandbox Stories',
          authors: ['Ada Lovelace', 'Alan Turing'],
          series: { name: 'Foundations' },
          cover: 'https://api.example-books.test/covers/b1.jpg',
          summary: 'A book about safe extensibility.',
          identifiers: { isbn_13: '9781234567897' },
        },
        { id: 'b2' }, // missing title -> skipped
        { title: 'No Id' }, // missing id -> skipped
        'not-an-object',
      ],
    };
    const books = mapDeclarativeCatalog(json, manifest());
    expect(books).toHaveLength(1);
    const b = books[0];
    expect(b!.id).toBe('b1');
    expect(b!.title).toBe('Sandbox Stories');
    expect(b!.authors).toEqual(['Ada Lovelace', 'Alan Turing']);
    expect(b!.series).toBe('Foundations');
    expect(b!.coverUrl).toBe('https://api.example-books.test/covers/b1.jpg');
    expect(b!.description).toBe('A book about safe extensibility.');
    expect(b!.mediaType).toBe('ebook');
    expect(b!.sourceProviderId).toBe('example-rest-catalog');
    expect(b!.identifiers).toEqual({ isbn_13: '9781234567897' });
  });

  it('splits a comma-separated author string', () => {
    const json = { results: [{ id: 'x', title: 'T', authors: 'A One, B Two' }] };
    expect(mapDeclarativeCatalog(json, manifest())[0]!.authors).toEqual(['A One', 'B Two']);
  });

  it('treats the body as the array when itemsPath is empty', () => {
    const m = manifest({
      catalog: { endpoint: '/api/books', fields: { id: 'id', title: 'title' } },
    });
    const books = mapDeclarativeCatalog([{ id: '1', title: 'Root Item' }], m);
    expect(books).toHaveLength(1);
    expect(books[0]!.title).toBe('Root Item');
  });

  it('stores a scalar identifier under isbn', () => {
    const m = manifest({
      catalog: {
        endpoint: '/x',
        fields: { id: 'id', title: 'title', identifiers: 'isbn' },
      },
    });
    const books = mapDeclarativeCatalog([{ id: '1', title: 'T', isbn: '9780000000001' }], m);
    expect(books[0]!.identifiers).toEqual({ isbn: '9780000000001' });
  });

  it('returns an empty list for a non-array items location', () => {
    expect(mapDeclarativeCatalog({ results: 'nope' }, manifest())).toEqual([]);
  });
});

describe('extractAuthors', () => {
  it('handles arrays, csv strings, and neither', () => {
    expect(extractAuthors(['A', ' B '])).toEqual(['A', 'B']);
    expect(extractAuthors('A, B')).toEqual(['A', 'B']);
    expect(extractAuthors(42)).toEqual(['42']);
    expect(extractAuthors(null)).toEqual([]);
  });
});

describe('joinUrl', () => {
  it('normalizes the slash between base and endpoint', () => {
    expect(joinUrl('https://x.test/', '/api')).toBe('https://x.test/api');
    expect(joinUrl('https://x.test', 'api')).toBe('https://x.test/api');
    expect(joinUrl('https://x.test/', '')).toBe('https://x.test');
  });
});

describe('createDeclarativePluginProvider', () => {
  it('sandbox-checks, fetches, and maps into books', async () => {
    const seen: string[] = [];
    const provider = createDeclarativePluginProvider(manifest(), {
      fetchJson: async (url) => {
        seen.push(url);
        return { results: [{ id: '1', title: 'The Pragmatic Plugin', authors: ['M. Author'] }] };
      },
    });
    expect(provider.id).toBe('example-rest-catalog');
    expect(provider.capabilities.has('catalog')).toBe(true);

    const books = await provider.listBooks();
    expect(seen).toEqual(['https://api.example-books.test/api/books']);
    expect(books).toHaveLength(1);
    expect(books[0]!.title).toBe('The Pragmatic Plugin');
    expect(books[0]!.sourceProviderId).toBe('example-rest-catalog');
  });

  it('never fetches a denied domain (typed error)', async () => {
    let fetched = false;
    const provider = createDeclarativePluginProvider(
      manifest({ baseUrl: 'https://evil.example.com' }),
      {
        fetchJson: async () => {
          fetched = true;
          return {};
        },
      },
    );
    await expect(provider.listBooks()).rejects.toMatchObject({ kind: 'domain-denied' });
    expect(fetched).toBe(false);
  });

  it('wraps a fetch failure in a typed PluginError', async () => {
    const provider = createDeclarativePluginProvider(manifest(), {
      fetchJson: async () => {
        throw new Error('boom');
      },
    });
    await expect(provider.listBooks()).rejects.toBeInstanceOf(PluginError);
  });
});
