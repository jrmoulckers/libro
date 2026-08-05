/**
 * The browser-native **WASM plugin runtime** — the second plugin kind, behind the
 * same boundary as the declarative {@link import('./engine')}.
 *
 * A WASM plugin ships a sandboxed `.wasm` module that implements the catalog
 * contract in code (for logic beyond declarative field-mapping). This is the
 * browser re-expression of the blueprint's `wasm.rs`, with one big simplification:
 * the module runs on the **native `WebAssembly` API** — no `wasmi` interpreter/host
 * is needed, because the browser (and Node) already ship a WASM engine. The ABI is
 * kept byte-compatible with the blueprint so the very same guest module runs here.
 *
 * # Sandbox (host-enforced — the guest is untrusted)
 * The module gets **no ambient capabilities**. Its only outside reach is the host
 * imports we provide. `host_http_get` first runs the exact same
 * {@link checkDomainAllowed} allowlist check as the declarative engine; a denied
 * host is **unreachable** (the host never fetches, the guest gets `-1`). Response,
 * output, URL, and config sizes are all bounded.
 *
 * # Limitation vs. the blueprint (documented, not hidden)
 * The blueprint's `wasmi` host uses **fuel metering** to bound execution so a
 * runaway/infinite-loop guest terminates instead of hanging. The native
 * `WebAssembly` engine exposes **no instruction/fuel primitive**, so this runtime
 * cannot interrupt an infinite loop in the guest. A malicious/buggy module that
 * never returns is therefore a **documented limitation** here (mitigated only by
 * running plugins off the critical path and by users vetting what they install) —
 * we do not claim to prevent it. Size bounds still protect against memory bombs.
 *
 * # ABI v1 (byte-compatible with the blueprint)
 * Strings/JSON cross as `(ptr, len)` into the guest's linear memory; a packed
 * `i64` return carries `(ptr << 32) | len` (see {@link packAbi}/{@link unpackAbi}).
 *
 * **Guest exports:** `memory`, `plugin_abi_version() -> i32`, `alloc(i32) -> i32`,
 * `list_catalog(cfg_ptr: i32, cfg_len: i32) -> i64`.
 *
 * **Host imports (module `env`):** `host_http_get(url_ptr, url_len) -> i64` (body
 * length, or `-1` on denial/error), `host_http_body(dst_ptr, dst_len) -> i32`,
 * `host_log(ptr, len)`.
 *
 * The pure pieces — {@link packAbi}, {@link unpackAbi}, {@link mapWasmBooks} — are
 * unit-tested directly; {@link runWasmCatalog} is exercised against the committed
 * example `.wasm` with a fake {@link HostHttp} (no network).
 */

import type { Book, MediaType } from '../models';
import type { Provider, ProviderCapability } from '../providers/types';
import { checkDomainAllowed } from './engine';
import { PluginError, WASM_ABI_VERSION, type WasmPluginManifest } from './manifest';

/** Max HTTP response body the host will hand back to the guest (8 MiB). */
const MAX_RESPONSE_BYTES = 8 * 1024 * 1024;
/** Max serialized guest output — the returned book-array JSON (8 MiB). */
const MAX_OUTPUT_BYTES = 8 * 1024 * 1024;
/** Max config JSON handed into the guest (64 KiB). */
const MAX_CONFIG_BYTES = 64 * 1024;
/** Max URL / log string length the guest may pass to a host import (8 KiB). */
const MAX_STRING_BYTES = 8 * 1024;

/**
 * The host HTTP seam: performs a GET **after** the allowlist check has passed. It
 * is **synchronous** because the guest calls `host_http_get` synchronously and
 * expects the body length back immediately (the WASM call stack cannot await).
 * Tests inject a fake that replays fixture bytes; the browser default uses a
 * synchronous `XMLHttpRequest` (see {@link browserSyncHttp}).
 */
export interface HostHttp {
  /** Fetch `url` (already allowlist-approved) and return the response bytes. */
  get(url: string): Uint8Array;
}

/** Options for {@link runWasmCatalog}. */
export interface RunWasmOptions {
  /** Provider id stamped as {@link Book.sourceProviderId}. */
  providerId: string;
  /** Default media type for produced books (a guest may override per book). */
  mediaType: MediaType;
  /** User config JSON handed to the guest (e.g. `{ base_url, api_key }`). */
  config: unknown;
  /** The manifest's allowlisted bare hosts — the network sandbox. */
  allowedDomains: readonly string[];
  /** The (synchronous) host HTTP seam. */
  http: HostHttp;
}

/** Pack a `(ptr, len)` pair into the ABI's `i64` as `(ptr << 32) | len`. */
export function packAbi(ptr: number, len: number): bigint {
  return (BigInt(ptr >>> 0) << 32n) | BigInt(len >>> 0);
}

/** Unpack the ABI's packed `i64` back into `{ ptr, len }`. */
export function unpackAbi(packed: bigint): { ptr: number; len: number } {
  return {
    ptr: Number((packed >> 32n) & 0xffff_ffffn),
    len: Number(packed & 0xffff_ffffn),
  };
}

/** A book record as produced by a WASM guest (before host-side normalization). */
interface WasmBook {
  id?: unknown;
  title?: unknown;
  authors?: unknown;
  series?: unknown;
  cover_url?: unknown;
  description?: unknown;
  identifiers?: unknown;
  media_type?: unknown;
}

/**
 * Parse the guest's JSON book array into normalized {@link Book}s. PURE + typed:
 * malformed JSON is a {@link PluginError} (`output`); an id/title-less record is
 * skipped rather than fatal — mirroring the declarative engine's robustness.
 */
export function mapWasmBooks(
  providerId: string,
  defaultMediaType: MediaType,
  jsonBytes: Uint8Array,
): Book[] {
  let raw: unknown;
  try {
    raw = JSON.parse(new TextDecoder().decode(jsonBytes));
  } catch (cause) {
    throw new PluginError('output', 'guest output is not valid JSON', cause);
  }
  if (!Array.isArray(raw)) {
    throw new PluginError('output', 'guest output is not a JSON array of books');
  }

  const books: Book[] = [];
  for (const entry of raw as WasmBook[]) {
    const id = asString(entry.id);
    const title = asString(entry.title);
    if (!id || !title) continue;

    const book: Book = {
      id,
      title,
      authors: Array.isArray(entry.authors)
        ? entry.authors.map(asString).filter((s): s is string => Boolean(s))
        : [],
      mediaType: asMediaType(entry.media_type) ?? defaultMediaType,
      sourceProviderId: providerId,
    };

    const series = asString(entry.series);
    if (series) book.series = series;
    const cover = asString(entry.cover_url);
    if (cover) book.coverUrl = cover;
    const description = asString(entry.description);
    if (description) book.description = description;

    if (
      entry.identifiers &&
      typeof entry.identifiers === 'object' &&
      !Array.isArray(entry.identifiers)
    ) {
      const identifiers: Record<string, string> = {};
      for (const [scheme, value] of Object.entries(entry.identifiers as Record<string, unknown>)) {
        const v = asString(value);
        if (v) identifiers[scheme] = v;
      }
      if (Object.keys(identifiers).length > 0) book.identifiers = identifiers;
    }

    books.push(book);
  }
  return books;
}

/**
 * Instantiate and run a WASM plugin's `list_catalog` against `config`, enforcing
 * the domain sandbox and size bounds, and mapping the returned JSON to normalized
 * {@link Book}s. The guest runs synchronously once instantiated; the injected
 * {@link HostHttp} seam is likewise synchronous. All failure modes are typed
 * {@link PluginError}s — a misbehaving guest never throws untyped.
 */
export async function runWasmCatalog(
  wasmBytes: BufferSource,
  options: RunWasmOptions,
): Promise<Book[]> {
  const state: { lastBody: Uint8Array<ArrayBufferLike>; httpError: string | null } = {
    lastBody: new Uint8Array(0),
    httpError: null,
  };
  const memRef: { m?: WebAssembly.Memory } = {};

  const bytesAt = (ptr: number, len: number): Uint8Array =>
    new Uint8Array((memRef.m as WebAssembly.Memory).buffer, ptr, len);
  const readString = (ptr: number, len: number): string =>
    new TextDecoder().decode(bytesAt(ptr, len).slice());

  const imports: WebAssembly.Imports = {
    env: {
      host_http_get: (urlPtr: number, urlLen: number): bigint => {
        if (urlLen < 0 || urlLen > MAX_STRING_BYTES) return -1n;
        const url = readString(urlPtr, urlLen);
        // SANDBOX: reuse the declarative engine's exact allowlist check.
        if (!checkDomainAllowed(url, options.allowedDomains)) {
          state.httpError = `network permission denied: '${url}' is not in allowedDomains`;
          return -1n;
        }
        try {
          const body = options.http.get(url);
          if (body.length > MAX_RESPONSE_BYTES) {
            state.httpError = 'http response exceeds the size limit';
            return -1n;
          }
          state.lastBody = body;
          return BigInt(body.length);
        } catch (cause) {
          state.httpError = cause instanceof Error ? cause.message : String(cause);
          return -1n;
        }
      },
      host_http_body: (dstPtr: number, dstLen: number): number => {
        if (dstPtr < 0 || dstLen < 0) return -1;
        const body = state.lastBody;
        state.lastBody = new Uint8Array(0);
        const n = Math.min(body.length, dstLen);
        new Uint8Array((memRef.m as WebAssembly.Memory).buffer).set(body.subarray(0, n), dstPtr);
        return n;
      },
      host_log: (ptr: number, len: number): void => {
        if (len < 0 || len > MAX_STRING_BYTES) return;
        try {
          readString(ptr, len);
        } catch {
          // Ignore an out-of-bounds log request from a misbehaving guest.
        }
      },
    },
  };

  let instance: WebAssembly.Instance;
  try {
    ({ instance } = await WebAssembly.instantiate(wasmBytes, imports));
  } catch (cause) {
    throw new PluginError('instantiate', 'wasm module failed to compile/instantiate', cause);
  }

  const exports = instance.exports;
  const memory = exports.memory;
  if (!(memory instanceof WebAssembly.Memory))
    throw new PluginError('missing-export', "missing export 'memory'");
  memRef.m = memory;
  requireFunc(exports, 'plugin_abi_version');
  const abiVersion = Number((exports.plugin_abi_version as () => number)());
  if (abiVersion !== WASM_ABI_VERSION) {
    throw new PluginError(
      'abi-mismatch',
      `wasm plugin ABI mismatch: module reports ${abiVersion}, host implements ${WASM_ABI_VERSION}`,
    );
  }
  const alloc = requireFunc(exports, 'alloc') as (len: number) => number;
  const listCatalog = requireFunc(exports, 'list_catalog') as (ptr: number, len: number) => bigint;

  const configBytes = new TextEncoder().encode(JSON.stringify(options.config ?? {}));
  if (configBytes.length > MAX_CONFIG_BYTES)
    throw new PluginError('output', 'config JSON too large');

  const cfgPtr = Number(alloc(configBytes.length));
  if (cfgPtr <= 0) throw new PluginError('memory', 'guest alloc returned a non-positive pointer');
  new Uint8Array(memory.buffer).set(configBytes, cfgPtr);

  const packed = listCatalog(cfgPtr, configBytes.length);

  // A host-side denial/error (e.g. a blocked domain) trumps the guest's result —
  // the denied fetch was unreachable, so surface it clearly.
  if (state.httpError) throw new PluginError('http', state.httpError);
  if (packed === 0n) throw new PluginError('output', 'guest signalled an error (null result)');

  const { ptr, len } = unpackAbi(packed);
  if (len > MAX_OUTPUT_BYTES)
    throw new PluginError('output', 'guest output exceeds the size limit');
  const out = bytesAt(ptr, len).slice();
  return mapWasmBooks(options.providerId, options.mediaType, out);
}

/**
 * A {@link Provider} backed by a sandboxed WASM module. `listBooks()` runs the
 * guest's `list_catalog` via {@link runWasmCatalog}, so it slots into the registry
 * identically to native connectors and the declarative plugin provider.
 */
export function createWasmPluginProvider(
  manifest: WasmPluginManifest,
  wasmBytes: BufferSource,
  deps: { config?: unknown; http?: HostHttp } = {},
): Provider {
  const capabilities: ReadonlySet<ProviderCapability> = new Set(manifest.capabilities);
  const mediaType = manifest.wasm.mediaType ?? 'ebook';
  const http = deps.http ?? browserSyncHttp;

  return {
    id: manifest.id,
    displayName: manifest.displayName,
    capabilities,
    async listBooks(): Promise<Book[]> {
      return runWasmCatalog(wasmBytes, {
        providerId: manifest.id,
        mediaType,
        config: deps.config ?? {},
        allowedDomains: manifest.allowedDomains,
        http,
      });
    },
  };
}

/**
 * The default browser {@link HostHttp}: a **synchronous** `XMLHttpRequest`. Sync
 * XHR blocks the calling thread, which is why plugins run off the critical path;
 * a worker-hosted async bridge is a documented TODO. Only used when a user opts
 * into a WASM plugin without supplying their own host; tests inject a fake.
 */
export const browserSyncHttp: HostHttp = {
  get(url: string): Uint8Array {
    const xhr = new XMLHttpRequest();
    xhr.open('GET', url, false);
    xhr.overrideMimeType('text/plain; charset=x-user-defined');
    xhr.send();
    if (xhr.status < 200 || xhr.status >= 300) {
      throw new Error(`unexpected status ${xhr.status}`);
    }
    const text = xhr.responseText;
    const bytes = new Uint8Array(text.length);
    for (let i = 0; i < text.length; i += 1) bytes[i] = text.charCodeAt(i) & 0xff;
    return bytes;
  },
};

function requireFunc(exports: WebAssembly.Exports, name: string): unknown {
  const fn = exports[name];
  if (typeof fn !== 'function') {
    throw new PluginError('missing-export', `wasm module is missing the required export '${name}'`);
  }
  return fn;
}

function asString(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  return trimmed ? trimmed : undefined;
}

function asMediaType(value: unknown): MediaType | undefined {
  const s = asString(value)?.toLowerCase();
  return s === 'ebook' || s === 'audiobook' || s === 'podcast' ? s : undefined;
}
