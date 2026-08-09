/**
 * The plugin SDK — third-party catalog connectors without forking Libro.
 *
 * A **plugin** lets a user add a new library source by supplying a small manifest
 * (plus, for the WASM kind, a `.wasm` module) from on-device config — no rebuild,
 * no server. This is the browser TypeScript re-expression of the blueprint's
 * `core/src/plugins/` (`mod.rs` manifest/validate, `engine.rs`, `wasm.rs`).
 *
 * # Two plugin kinds behind one boundary
 * A manifest is **exactly one kind**:
 *  - `declarative` — a JSON catalog spec (baseUrl + endpoint + dotted field
 *    selectors) interpreted by {@link import('./engine')} with zero code. Handles
 *    the common REST/JSON-catalog case.
 *  - `wasm` — a sandboxed `.wasm` module that implements the catalog contract in
 *    code (see {@link import('./wasm')}), for logic beyond field-mapping.
 * Both validate through the same rules, present as the same {@link Provider}, and
 * are sandboxed to the manifest's {@link PluginManifest.allowedDomains}.
 *
 * # Security boundary
 * A plugin may only reach **explicitly declared** domains. The declarative engine
 * and the WASM host both deny any request whose host is not on the allowlist
 * (see `checkDomainAllowed`), enforced host-side before any fetch. The loader here
 * rejects malformed or over-broad manifests (wildcards, schemes, ports).
 *
 * Everything in this module is PURE (no network, no DOM): parsing a `.wasm` and
 * performing the actual `fetch` live in the deps-injected shells of `engine.ts`
 * and `wasm.ts`.
 */

import type { MediaType } from '../models';
import { MEDIA_TYPES } from '../models';
import type { ProviderCapability } from '../providers/types';
import { PROVIDER_CAPABILITIES } from '../providers/types';

/** The plugin manifest schema version this build implements. */
export const PLUGIN_API_VERSION = 1;

/** The WASM guest ABI version this host implements (see {@link import('./wasm')}). */
export const WASM_ABI_VERSION = 1;

/** The kind of a plugin manifest. Exactly one kind is present per manifest. */
export type PluginKind = 'declarative' | 'wasm';

/** Kinds of failure from loading, validating, or running a plugin. */
export type PluginErrorKind =
  | 'parse'
  | 'invalid'
  | 'domain-denied'
  | 'instantiate'
  | 'abi-mismatch'
  | 'missing-export'
  | 'output'
  | 'memory'
  | 'http';

/** A typed plugin error — the loader/engine/runtime never throw untyped. */
export class PluginError extends Error {
  constructor(
    readonly kind: PluginErrorKind,
    message: string,
    override readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'PluginError';
  }
}

/**
 * How to map each JSON response item onto a normalized {@link Book}. Each value
 * is a simple dotted JSON path into a response item (e.g. `"series.name"`).
 */
export interface PluginFieldMap {
  /** Dotted path to the item's stable id (required). */
  id: string;
  /** Dotted path to the title (required). */
  title: string;
  /** Dotted path to authors (array of scalars, or a comma-separated string). */
  authors?: string;
  /** Dotted path to the series name. */
  series?: string;
  /** Dotted path to a cover image URL. */
  cover?: string;
  /** Dotted path to a description/synopsis. */
  description?: string;
  /**
   * Dotted path to identifiers. Resolves to either an object of
   * `scheme -> value` (each scalar entry copied into {@link Book.identifiers}) or
   * a single scalar, stored as `{ isbn: value }`.
   */
  identifiers?: string;
}

/** The declarative catalog spec: how to fetch the library and map it to Books. */
export interface DeclarativeCatalogSpec {
  /** Path appended to {@link DeclarativePluginManifest.baseUrl} (e.g. `/api/books`). */
  endpoint: string;
  /** Dotted path to the array of items; empty/omitted means the body is the array. */
  itemsPath?: string;
  /** Media type stamped on produced books. Defaults to `ebook`. */
  mediaType?: MediaType;
  /** Field selectors mapping response items onto {@link Book}s. */
  fields: PluginFieldMap;
}

/** The WASM-kind spec (the `.wasm` bytes are supplied separately at load time). */
export interface WasmSpec {
  /** Guest ABI version; if declared it must equal {@link WASM_ABI_VERSION}. */
  abiVersion?: number;
  /** Default media type stamped on produced books. Defaults to `ebook`. */
  mediaType?: MediaType;
}

/** Fields common to both plugin kinds. */
interface PluginManifestBase {
  /** Stable machine id, used as the provider type + {@link Book.sourceProviderId}. */
  id: string;
  /** Human-friendly name for the UI. */
  displayName: string;
  /**
   * Allowlisted bare hosts. A request host matches an entry if it equals it or is
   * a subdomain of it (`api.example.com` matches `example.com`). No wildcards,
   * schemes, paths, or ports.
   */
  allowedDomains: string[];
  /** Requested capabilities; always includes `catalog`. */
  capabilities: ProviderCapability[];
  version?: string;
  author?: string;
}

/** A declarative (JSON-catalog) plugin manifest. */
export interface DeclarativePluginManifest extends PluginManifestBase {
  kind: 'declarative';
  /** Server root the endpoint is resolved against (e.g. `https://api.example.com`). */
  baseUrl: string;
  catalog: DeclarativeCatalogSpec;
}

/** A WASM (sandboxed-module) plugin manifest. */
export interface WasmPluginManifest extends PluginManifestBase {
  kind: 'wasm';
  wasm: WasmSpec;
}

/** A validated plugin manifest — exactly one kind. */
export type PluginManifest = DeclarativePluginManifest | WasmPluginManifest;

/**
 * Validate a parsed-JSON value into a typed {@link PluginManifest}, rejecting
 * malformed or over-broad definitions. PURE — no network, no I/O.
 *
 * Enforced invariants (mirroring the blueprint):
 *  - `id` present, non-empty, no whitespace; `displayName` present;
 *  - at least one `allowedDomains` entry, each a bare host (no wildcard/scheme/
 *    path/port);
 *  - **exactly one** kind — either `catalog` (declarative) or `wasm`, never both
 *    and never neither; an explicit `kind` field, if present, must agree;
 *  - declarative: non-empty `baseUrl` and `catalog.fields.id`/`.title`;
 *  - wasm: `wasm.abiVersion`, if declared, equals {@link WASM_ABI_VERSION}.
 *
 * @throws {PluginError} of kind `invalid` (or `parse` if the value is not an
 * object) on any violation.
 */
export function validatePluginManifest(value: unknown): PluginManifest {
  const obj = asRecord(value);
  if (!obj) {
    throw new PluginError('parse', 'manifest must be a JSON object');
  }

  const id = asString(obj.id);
  if (!id) throw new PluginError('invalid', 'id is empty');
  if (/\s/.test(id)) throw new PluginError('invalid', 'id must not contain whitespace');

  const displayName = asString(obj.displayName) ?? asString(obj.name);
  if (!displayName) throw new PluginError('invalid', 'displayName is empty');

  const allowedDomains = asArray(obj.allowedDomains)?.map((d) => asString(d) ?? '') ?? [];
  if (allowedDomains.length === 0) {
    throw new PluginError('invalid', 'allowedDomains must list at least one domain');
  }
  for (const domain of allowedDomains) {
    validateDomain(domain);
  }

  const capabilities = normalizeCapabilities(obj.capabilities);

  const hasCatalog = obj.catalog != null;
  const hasWasm = obj.wasm != null;
  if (hasCatalog && hasWasm) {
    throw new PluginError(
      'invalid',
      "manifest declares both 'catalog' and 'wasm' (exactly one kind is required)",
    );
  }
  if (!hasCatalog && !hasWasm) {
    throw new PluginError(
      'invalid',
      "manifest declares neither 'catalog' nor 'wasm' (exactly one kind is required)",
    );
  }
  const kind: PluginKind = hasCatalog ? 'declarative' : 'wasm';
  const declaredKind = asString(obj.kind);
  if (declaredKind && declaredKind !== kind) {
    throw new PluginError(
      'invalid',
      `kind '${declaredKind}' disagrees with the present spec ('${kind}')`,
    );
  }

  const base: PluginManifestBase = {
    id,
    displayName,
    allowedDomains,
    capabilities,
    version: asString(obj.version),
    author: asString(obj.author),
  };

  if (kind === 'declarative') {
    return { ...base, kind, ...validateDeclarative(obj.catalog, obj.baseUrl) };
  }
  return { ...base, kind, wasm: validateWasm(obj.wasm) };
}

function validateDeclarative(
  catalogValue: unknown,
  baseUrlValue: unknown,
): Pick<DeclarativePluginManifest, 'baseUrl' | 'catalog'> {
  const baseUrl = asString(baseUrlValue);
  if (!baseUrl)
    throw new PluginError('invalid', 'declarative manifest requires a non-empty baseUrl');

  const catalog = asRecord(catalogValue);
  if (!catalog) throw new PluginError('invalid', 'catalog must be an object');

  const fields = asRecord(catalog.fields);
  const fieldId = fields && asString(fields.id);
  const fieldTitle = fields && asString(fields.title);
  if (!fieldId || !fieldTitle) {
    throw new PluginError('invalid', 'catalog.fields.id and catalog.fields.title are required');
  }

  const mediaType = asMediaType(catalog.mediaType);

  return {
    baseUrl,
    catalog: {
      endpoint: asString(catalog.endpoint) ?? '',
      itemsPath: asString(catalog.itemsPath),
      mediaType,
      fields: {
        id: fieldId,
        title: fieldTitle,
        authors: asString(fields.authors),
        series: asString(fields.series),
        cover: asString(fields.cover),
        description: asString(fields.description),
        identifiers: asString(fields.identifiers),
      },
    },
  };
}

function validateWasm(wasmValue: unknown): WasmSpec {
  const wasm = asRecord(wasmValue);
  if (!wasm) throw new PluginError('invalid', 'wasm must be an object');

  const abiVersion = typeof wasm.abiVersion === 'number' ? wasm.abiVersion : undefined;
  if (abiVersion !== undefined && abiVersion !== WASM_ABI_VERSION) {
    throw new PluginError(
      'invalid',
      `wasm.abiVersion ${abiVersion} is unsupported (this build implements ${WASM_ABI_VERSION})`,
    );
  }
  return { abiVersion, mediaType: asMediaType(wasm.mediaType) };
}

/**
 * Reject over-broad or malformed allowlist entries: a bare host only — no
 * wildcards, scheme, path, or port, and it must contain a dot.
 */
export function validateDomain(domain: string): void {
  const d = domain.trim();
  if (!d) throw new PluginError('invalid', 'empty allowed domain');
  if (d.includes('*')) {
    throw new PluginError(
      'invalid',
      `over-broad allowed domain '${d}' (wildcards are not permitted)`,
    );
  }
  if (d.includes('://') || d.includes('/') || d.includes(':')) {
    throw new PluginError(
      'invalid',
      `allowed domain '${d}' must be a bare host (no scheme, path, or port)`,
    );
  }
  if (!d.includes('.')) {
    throw new PluginError('invalid', `allowed domain '${d}' is not a valid host`);
  }
}

function normalizeCapabilities(value: unknown): ProviderCapability[] {
  const raw = asArray(value) ?? [];
  const out = new Set<ProviderCapability>(['catalog']);
  for (const entry of raw) {
    const cap = asString(entry);
    if (cap && (PROVIDER_CAPABILITIES as readonly string[]).includes(cap)) {
      out.add(cap as ProviderCapability);
    }
  }
  return [...out];
}

function asMediaType(value: unknown): MediaType | undefined {
  const s = asString(value)?.toLowerCase();
  return s && (MEDIA_TYPES as readonly string[]).includes(s) ? (s as MediaType) : undefined;
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function asArray(value: unknown): unknown[] | undefined {
  return Array.isArray(value) ? value : undefined;
}

function asString(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  return trimmed ? trimmed : undefined;
}
