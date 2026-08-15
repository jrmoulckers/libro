/// <reference types="node" />
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { loadPlugins, type PluginEntry } from './load';

function fixtureWasm(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(
    readFileSync(join(process.cwd(), 'src/lib/plugins/fixtures/example-catalog.wasm')),
  );
}

const DECLARATIVE_MANIFEST = {
  id: 'example-rest-catalog',
  displayName: 'Example REST Catalog',
  kind: 'declarative',
  baseUrl: 'https://api.example-books.test',
  allowedDomains: ['api.example-books.test'],
  capabilities: ['catalog'],
  catalog: {
    endpoint: '/api/books',
    itemsPath: 'results',
    fields: { id: 'id', title: 'title', authors: 'authors' },
  },
};

const WASM_MANIFEST = {
  id: 'example-wasm-catalog',
  displayName: 'Example WASM Catalog',
  kind: 'wasm',
  allowedDomains: ['api.wasm-books.test'],
  capabilities: ['catalog'],
  wasm: { abiVersion: 1, mediaType: 'ebook' },
};

describe('loadPlugins', () => {
  it('builds a declarative provider from a valid manifest', async () => {
    const { providers, errors } = loadPlugins([{ manifest: DECLARATIVE_MANIFEST }], {
      fetchJson: async () => ({ results: [{ id: '1', title: 'Loaded Book' }] }),
    });
    expect(errors).toEqual([]);
    expect(providers).toHaveLength(1);
    const books = await providers[0]!.listBooks!();
    expect(books[0]!.title).toBe('Loaded Book');
  });

  it('builds a wasm provider from a manifest + bytes', async () => {
    const { providers, errors } = loadPlugins(
      [
        {
          manifest: WASM_MANIFEST,
          wasmBytes: fixtureWasm(),
          config: { base_url: 'https://api.wasm-books.test' },
        },
      ],
      {
        http: {
          get: () =>
            new TextEncoder().encode(JSON.stringify({ items: [{ guid: 'a1', name: 'W' }] })),
        },
      },
    );
    expect(errors).toEqual([]);
    const books = await providers[0]!.listBooks!();
    expect(books[0]!.id).toBe('a1');
  });

  it('records an invalid manifest as an error without aborting the batch', () => {
    const entries: PluginEntry[] = [
      { manifest: { id: 'bad' } }, // no kind / spec
      { manifest: DECLARATIVE_MANIFEST },
    ];
    const { providers, errors } = loadPlugins(entries);
    expect(providers).toHaveLength(1);
    expect(errors).toHaveLength(1);
    expect(errors[0]!.id).toBe('bad');
    expect(errors[0]!.error!.kind).toBe('invalid');
  });

  it('errors a wasm entry with no module bytes', () => {
    const { providers, errors } = loadPlugins([{ manifest: WASM_MANIFEST }]);
    expect(providers).toEqual([]);
    expect(errors[0]!.error!.message).toMatch(/missing its module bytes/);
  });
});
