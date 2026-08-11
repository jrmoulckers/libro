import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  validatePluginManifest,
  validateDomain,
  PluginError,
  WASM_ABI_VERSION,
  type DeclarativePluginManifest,
  type WasmPluginManifest,
} from './manifest';

function declarativeJson(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
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
      fields: { id: 'id', title: 'title', authors: 'authors' },
    },
    ...overrides,
  };
}

function wasmJson(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: 'example-wasm-catalog',
    displayName: 'Example WASM Catalog',
    kind: 'wasm',
    allowedDomains: ['api.wasm-books.test'],
    capabilities: ['catalog'],
    wasm: { abiVersion: 1, mediaType: 'ebook' },
    ...overrides,
  };
}

describe('validatePluginManifest', () => {
  it('accepts a well-formed declarative manifest', () => {
    const m = validatePluginManifest(declarativeJson()) as DeclarativePluginManifest;
    expect(m.kind).toBe('declarative');
    expect(m.baseUrl).toBe('https://api.example-books.test');
    expect(m.catalog.fields.id).toBe('id');
    expect(m.capabilities).toContain('catalog');
  });

  it('accepts a well-formed wasm manifest', () => {
    const m = validatePluginManifest(wasmJson()) as WasmPluginManifest;
    expect(m.kind).toBe('wasm');
    expect(m.wasm.abiVersion).toBe(WASM_ABI_VERSION);
    expect(m.wasm.mediaType).toBe('ebook');
  });

  it('infers kind from the present spec when `kind` is omitted', () => {
    const json = declarativeJson();
    delete json.kind;
    expect(validatePluginManifest(json).kind).toBe('declarative');
  });

  it('rejects a manifest declaring both kinds', () => {
    const json = declarativeJson({ wasm: { abiVersion: 1 } });
    expect(() => validatePluginManifest(json)).toThrow(/exactly one kind/);
  });

  it('rejects a manifest declaring neither kind', () => {
    const json = declarativeJson();
    delete json.catalog;
    delete json.kind;
    expect(() => validatePluginManifest(json)).toThrow(/exactly one kind/);
  });

  it('rejects an explicit kind that disagrees with the present spec', () => {
    const json = declarativeJson({ kind: 'wasm' });
    expect(() => validatePluginManifest(json)).toThrow(/disagrees/);
  });

  it('rejects an empty or whitespace-containing id', () => {
    expect(() => validatePluginManifest(declarativeJson({ id: '' }))).toThrow(/id is empty/);
    expect(() => validatePluginManifest(declarativeJson({ id: 'a b' }))).toThrow(/whitespace/);
  });

  it('requires at least one allowed domain', () => {
    expect(() => validatePluginManifest(declarativeJson({ allowedDomains: [] }))).toThrow(
      /at least one domain/,
    );
  });

  it('requires declarative id/title field selectors', () => {
    const json = declarativeJson({
      catalog: { endpoint: '/x', fields: { id: '', title: 't' } },
    });
    expect(() => validatePluginManifest(json)).toThrow(/id and catalog.fields.title/);
  });

  it('requires a non-empty baseUrl for declarative manifests', () => {
    expect(() => validatePluginManifest(declarativeJson({ baseUrl: '' }))).toThrow(/baseUrl/);
  });

  it('rejects an unsupported wasm ABI version', () => {
    expect(() => validatePluginManifest(wasmJson({ wasm: { abiVersion: 99 } }))).toThrow(
      /abiVersion 99 is unsupported/,
    );
  });

  it('always includes the catalog capability even if omitted', () => {
    const json = declarativeJson();
    delete json.capabilities;
    expect(validatePluginManifest(json).capabilities).toEqual(['catalog']);
  });

  it('merges recognized extra capabilities and drops unknown ones', () => {
    const m = validatePluginManifest(
      declarativeJson({ capabilities: ['progress-sync', 'not-a-cap'] }),
    );
    expect(m.capabilities).toContain('catalog');
    expect(m.capabilities).toContain('progress-sync');
    expect(m.capabilities).not.toContain('not-a-cap');
  });

  it('throws a typed PluginError on a non-object', () => {
    try {
      validatePluginManifest('nope');
      expect.unreachable();
    } catch (err) {
      expect(err).toBeInstanceOf(PluginError);
      expect((err as PluginError).kind).toBe('parse');
    }
  });
});

describe('validateDomain', () => {
  it('accepts a bare host', () => {
    expect(() => validateDomain('api.example.com')).not.toThrow();
  });

  it('rejects wildcards', () => {
    expect(() => validateDomain('*.example.com')).toThrow(/wildcards/);
  });

  it('rejects schemes, paths, and ports', () => {
    expect(() => validateDomain('https://example.com')).toThrow(/bare host/);
    expect(() => validateDomain('example.com/x')).toThrow(/bare host/);
    expect(() => validateDomain('example.com:8080')).toThrow(/bare host/);
  });

  it('rejects a host with no dot', () => {
    expect(() => validateDomain('localhost')).toThrow(/valid host/);
  });
});

describe('committed fixture manifests', () => {
  // The fixtures README states these validate against validatePluginManifest, but nothing
  // loaded them, so the claim was unenforced and they could drift from the schema silently.
  // They are read rather than inlined here deliberately: an inlined copy would re-introduce
  // the same gap.
  for (const file of ['example-rest-catalog.plugin.json', 'example-wasm-catalog.plugin.json']) {
    it(`validates ${file}`, () => {
      const raw = readFileSync(join(process.cwd(), 'src/lib/plugins/fixtures', file), 'utf8');
      expect(() => validatePluginManifest(JSON.parse(raw))).not.toThrow();
    });
  }
});
