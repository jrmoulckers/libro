/**
 * Plugin loading + registry composition — the browser analog of the blueprint's
 * `build_providers`.
 *
 * {@link loadPlugins} takes user-supplied plugin entries (a parsed manifest plus,
 * for the WASM kind, the module bytes and the user's config), validates each
 * through {@link validatePluginManifest}, branches on kind to build the right
 * {@link Provider} (declarative or WASM), and returns the providers alongside any
 * per-entry failures. It is **failure-isolated**: one malformed/invalid entry is
 * skipped into `errors`, never aborting the load — mirroring the blueprint's
 * "one bad plugin can never crash discovery" guarantee.
 *
 * ## How a user adds a plugin (on-device, no secrets)
 * There is no server and no registry to fetch from. A settings/UI layer collects,
 * from on-device storage, one {@link PluginEntry} per installed plugin — the
 * manifest JSON the user pasted/imported, plus (for a WASM plugin) the `.wasm`
 * bytes they supplied and any config values (base URL, API token) the manifest's
 * fields need — then calls {@link import('../library').registryWithPlugins} to
 * fold the resulting providers into the app registry. Nothing is baked into the
 * default build; `defaultRegistry` stays mock-only.
 */

import type { Provider } from '../providers/types';
import { createDeclarativePluginProvider, type DeclarativePluginDeps } from './engine';
import { createWasmPluginProvider, type HostHttp } from './wasm';
import { PluginError, validatePluginManifest } from './manifest';

/** One installed plugin, as collected from on-device config. */
export interface PluginEntry {
  /** The parsed manifest JSON (validated by {@link loadPlugins}). */
  manifest: unknown;
  /** The `.wasm` module bytes — required for a `wasm`-kind manifest. */
  wasmBytes?: BufferSource;
  /** User config handed to a WASM guest (e.g. `{ base_url, api_key }`). */
  config?: unknown;
}

/** Deps threaded into the built providers (the network seams). */
export interface LoadPluginsDeps {
  /** Declarative fetch seam (deps-injected; defaults to `fetch`). */
  fetchJson?: DeclarativePluginDeps['fetchJson'];
  /** WASM host HTTP seam (deps-injected; defaults to sync `XMLHttpRequest`). */
  http?: HostHttp;
}

/** A per-entry load failure (never aborts the whole load). */
export interface PluginLoadError {
  /** The offending manifest's id when known, else `undefined`. */
  id?: string;
  error: PluginError;
}

/** Result of {@link loadPlugins}: the built providers plus any per-entry failures. */
export interface LoadPluginsResult {
  providers: Provider[];
  errors: PluginLoadError[];
}

/**
 * Validate each entry, branch on kind, and build a {@link Provider} per plugin.
 * Failure-isolated: a bad manifest (or a WASM entry with no bytes) is recorded in
 * `errors` and skipped, never throwing.
 */
export function loadPlugins(
  entries: readonly PluginEntry[],
  deps: LoadPluginsDeps = {},
): LoadPluginsResult {
  const providers: Provider[] = [];
  const errors: PluginLoadError[] = [];

  for (const entry of entries) {
    try {
      const manifest = validatePluginManifest(entry.manifest);
      if (manifest.kind === 'declarative') {
        providers.push(createDeclarativePluginProvider(manifest, { fetchJson: deps.fetchJson }));
      } else {
        if (!entry.wasmBytes) {
          throw new PluginError(
            'invalid',
            `wasm plugin '${manifest.id}' is missing its module bytes`,
          );
        }
        providers.push(
          createWasmPluginProvider(manifest, entry.wasmBytes, {
            config: entry.config,
            http: deps.http,
          }),
        );
      }
    } catch (cause) {
      const error =
        cause instanceof PluginError
          ? cause
          : new PluginError('invalid', 'failed to load plugin', cause);
      errors.push({ id: idOf(entry.manifest), error });
    }
  }

  return { providers, errors };
}

function idOf(manifest: unknown): string | undefined {
  if (typeof manifest === 'object' && manifest !== null && 'id' in manifest) {
    const id = (manifest as { id: unknown }).id;
    return typeof id === 'string' ? id : undefined;
  }
  return undefined;
}
