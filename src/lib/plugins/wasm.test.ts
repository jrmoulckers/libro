/// <reference types="node" />
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  packAbi,
  unpackAbi,
  mapWasmBooks,
  runWasmCatalog,
  createWasmPluginProvider,
  type HostHttp,
} from './wasm';
import { PluginError, validatePluginManifest, type WasmPluginManifest } from './manifest';

/** The committed example guest (blueprint's own `example-wasm-catalog.wasm`). */
function fixtureWasm(): Uint8Array<ArrayBuffer> {
  return new Uint8Array(
    readFileSync(join(process.cwd(), 'src/lib/plugins/fixtures/example-catalog.wasm')),
  );
}

/** A fake host that replays fixture bytes and records the URLs it was asked for. */
function fakeHttp(body: string): HostHttp & { seen: string[] } {
  const seen: string[] = [];
  return {
    seen,
    get(url: string): Uint8Array {
      seen.push(url);
      return new TextEncoder().encode(body);
    },
  };
}

/** The server's NON-normalized shape; the guest maps it to normalized books. */
const SERVER_JSON = JSON.stringify({
  items: [
    { guid: 'a1', name: 'Wasm and Order', writer: 'G. Guest', isbn13: '9780000000009' },
    { guid: 'a2', name: 'The Interpreter', writer: 'P. Pure' },
  ],
});

const CONFIG = { base_url: 'https://api.wasm-books.test', api_key: 'K' };

describe('packAbi / unpackAbi', () => {
  it('round-trips a (ptr, len) pair through the packed i64', () => {
    expect(unpackAbi(packAbi(0x1234, 56))).toEqual({ ptr: 0x1234, len: 56 });
    expect(unpackAbi(packAbi(0xdead_beef, 0xffff))).toEqual({ ptr: 0xdead_beef, len: 0xffff });
    expect(packAbi(0, 5)).toBe(5n);
  });
});

describe('mapWasmBooks', () => {
  it('skips id/title-less records and parses identifiers + media override', () => {
    const json = new TextEncoder().encode(
      JSON.stringify([
        {
          id: 'x',
          title: 'T',
          authors: ['A'],
          identifiers: { asin: 'B000' },
          media_type: 'audiobook',
        },
        { id: '', title: 'no id' },
        { title: 'no id field' },
      ]),
    );
    const books = mapWasmBooks('p', 'ebook', json);
    expect(books).toHaveLength(1);
    expect(books[0]!.mediaType).toBe('audiobook');
    expect(books[0]!.identifiers).toEqual({ asin: 'B000' });
    expect(books[0]!.sourceProviderId).toBe('p');
  });

  it('throws a typed error on non-JSON output', () => {
    try {
      mapWasmBooks('p', 'ebook', new TextEncoder().encode('not json'));
      expect.unreachable();
    } catch (err) {
      expect(err).toBeInstanceOf(PluginError);
      expect((err as PluginError).kind).toBe('output');
    }
  });

  it('throws a typed error when output is not an array', () => {
    expect(() => mapWasmBooks('p', 'ebook', new TextEncoder().encode('{}'))).toThrow(PluginError);
  });
});

describe('runWasmCatalog against the committed fixture', () => {
  it('runs the guest and maps books over a fake http host', async () => {
    const http = fakeHttp(SERVER_JSON);
    const books = await runWasmCatalog(fixtureWasm(), {
      providerId: 'example-wasm-catalog',
      mediaType: 'ebook',
      config: CONFIG,
      allowedDomains: ['api.wasm-books.test'],
      http,
    });

    expect(books).toHaveLength(2);
    expect(books[0]!.id).toBe('a1');
    expect(books[0]!.title).toBe('Wasm and Order');
    expect(books[0]!.authors).toEqual(['G. Guest']);
    expect(books[0]!.sourceProviderId).toBe('example-wasm-catalog');
    expect(books[0]!.identifiers).toEqual({ isbn: '9780000000009' });
    // The guest built the URL from base_url and the host performed the fetch.
    expect(http.seen).toEqual(['https://api.wasm-books.test/api/books']);
  });

  it('makes a denied domain unreachable from the guest', async () => {
    const http = fakeHttp(SERVER_JSON);
    await expect(
      runWasmCatalog(fixtureWasm(), {
        providerId: 'example-wasm-catalog',
        mediaType: 'ebook',
        config: { base_url: 'https://evil.example.com', api_key: 'K' },
        allowedDomains: ['api.wasm-books.test'],
        http,
      }),
    ).rejects.toMatchObject({ kind: 'http' });
    // The host never fetched the disallowed URL.
    expect(http.seen).toEqual([]);
  });

  it('exposes a Provider via createWasmPluginProvider', async () => {
    const manifest = validatePluginManifest({
      id: 'example-wasm-catalog',
      displayName: 'Example WASM Catalog',
      kind: 'wasm',
      allowedDomains: ['api.wasm-books.test'],
      capabilities: ['catalog'],
      wasm: { abiVersion: 1, mediaType: 'ebook' },
    }) as WasmPluginManifest;

    const provider = createWasmPluginProvider(manifest, fixtureWasm(), {
      config: CONFIG,
      http: fakeHttp(SERVER_JSON),
    });
    expect(provider.capabilities.has('catalog')).toBe(true);
    const books = await provider.listBooks();
    expect(books.map((b) => b.id)).toEqual(['a1', 'a2']);
  });
});

// ---- edge-case modules hand-assembled to WASM bytes (no wasm toolchain) ------

describe('runWasmCatalog edge cases', () => {
  it('rejects an ABI version mismatch', async () => {
    const wasm = buildModule({ abiVersion: 99 });
    await expect(runWasmCatalog(wasm, edgeOpts())).rejects.toMatchObject({ kind: 'abi-mismatch' });
  });

  it('rejects a module missing a required export', async () => {
    const wasm = buildModule({ abiVersion: 1 }); // no alloc / list_catalog
    await expect(runWasmCatalog(wasm, edgeOpts())).rejects.toMatchObject({
      kind: 'missing-export',
    });
  });

  it('surfaces malformed guest output as a typed error, not a crash', async () => {
    // list_catalog returns packed(ptr=0, len=5) pointing at the bytes "hello".
    const wasm = buildModule({
      abiVersion: 1,
      allocPtr: 1024,
      listPacked: 5,
      data: 'hello',
    });
    await expect(runWasmCatalog(wasm, edgeOpts())).rejects.toMatchObject({ kind: 'output' });
  });

  it('reports a null guest result as an error', async () => {
    const wasm = buildModule({ abiVersion: 1, allocPtr: 1024, listPacked: 0 });
    await expect(runWasmCatalog(wasm, edgeOpts())).rejects.toMatchObject({ kind: 'output' });
  });
});

function edgeOpts() {
  return {
    providerId: 'edge',
    mediaType: 'ebook' as const,
    config: CONFIG,
    allowedDomains: ['api.wasm-books.test'],
    http: fakeHttp('{}'),
  };
}

// --- a tiny WASM binary encoder, just enough for the edge-case modules --------

function uleb(n: number): number[] {
  const out: number[] = [];
  do {
    let byte = n & 0x7f;
    n >>>= 7;
    if (n) byte |= 0x80;
    out.push(byte);
  } while (n);
  return out;
}

function sleb(value: number): number[] {
  const out: number[] = [];
  let n = value;
  for (;;) {
    const byte = n & 0x7f;
    n >>= 7;
    const done = (n === 0 && (byte & 0x40) === 0) || (n === -1 && (byte & 0x40) !== 0);
    out.push(done ? byte : byte | 0x80);
    if (done) return out;
  }
}

function vec(items: number[][]): number[] {
  return [...uleb(items.length), ...items.flat()];
}

function section(id: number, body: number[]): number[] {
  return [id, ...uleb(body.length), ...body];
}

interface ModuleSpec {
  abiVersion: number;
  allocPtr?: number;
  listPacked?: number;
  data?: string;
}

function buildModule(spec: ModuleSpec): Uint8Array<ArrayBuffer> {
  const hasAlloc = spec.allocPtr !== undefined;
  const hasList = spec.listPacked !== undefined;

  // Types: 0: ()->i32, 1: (i32)->i32, 2: (i32,i32)->i64.
  const types = section(
    1,
    vec([
      [0x60, 0x00, 0x01, 0x7f],
      [0x60, 0x01, 0x7f, 0x01, 0x7f],
      [0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e],
    ]),
  );

  const funcTypeIndices: number[][] = [[0]];
  if (hasAlloc) funcTypeIndices.push([1]);
  if (hasList) funcTypeIndices.push([2]);
  const functions = section(3, vec(funcTypeIndices));

  const memory = section(5, vec([[0x00, 0x01]]));

  const exportEntries: number[][] = [
    [...name('memory'), 0x02, 0x00],
    [...name('plugin_abi_version'), 0x00, 0x00],
  ];
  // Function indices: func0 = plugin_abi_version, then alloc, then list_catalog.
  if (hasAlloc) exportEntries.push([...name('alloc'), 0x00, 1]);
  if (hasList) exportEntries.push([...name('list_catalog'), 0x00, hasAlloc ? 2 : 1]);
  const exportSection = section(7, vec(exportEntries));

  const bodies: number[][] = [funcBody([0x41, ...sleb(spec.abiVersion)])];
  if (hasAlloc) bodies.push(funcBody([0x41, ...sleb(spec.allocPtr as number)]));
  if (hasList) bodies.push(funcBody([0x42, ...sleb(spec.listPacked as number)]));
  const code = section(10, vec(bodies));

  const sections = [types, functions, memory, exportSection, code];
  if (spec.data !== undefined) {
    const bytes = [...new TextEncoder().encode(spec.data)];
    const segment = [0x00, 0x41, ...sleb(0), 0x0b, ...uleb(bytes.length), ...bytes];
    sections.push(section(11, vec([segment])));
  }

  return new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, ...sections.flat()]);
}

function funcBody(instrs: number[]): number[] {
  const code = [0x00, ...instrs, 0x0b]; // 0 locals, instrs, end
  return [...uleb(code.length), ...code];
}

function name(s: string): number[] {
  const bytes = [...new TextEncoder().encode(s)];
  return [...uleb(bytes.length), ...bytes];
}
