/**
 * Composition root for the library pipeline: providers -> aggregation -> index.
 *
 * Keeps the wiring out of the Svelte component so it stays unit-testable and so
 * Phase 2 can swap the default provider set (add OPDS/Audiobookshelf/…) in one
 * place. The mock provider is the only source for now.
 */

import { InMemoryLibraryIndex } from './index/memory';
import { IdbLibraryIndex, idbAvailable } from './index/idb';
import type { LibraryIndex } from './index/types';
import type { Book } from './models';
import { createMockProvider } from './providers/mock';
import { createOpdsProvider, type OpdsConfig } from './providers/opds';
import { aggregateLibrary, ProviderRegistry } from './providers/registry';
import type { ProviderRunError } from './providers/registry';

/**
 * The providers the app pulls from. The mock provider is the only source for now;
 * real connectors are registered here as later phases land (see
 * {@link registryWithOpds} for the config-driven pattern).
 */
export function defaultRegistry(): ProviderRegistry {
  return new ProviderRegistry([createMockProvider()]);
}

/**
 * Example of config-driven registration of the real OPDS connector, kept separate
 * from {@link defaultRegistry} so no live credentials are baked into the default
 * pipeline. A UI/settings layer would collect {@link OpdsConfig}s from on-device
 * storage (there is no server to hold secrets) and call this:
 *
 * ```ts
 * const registry = registryWithOpds([
 *   { id: 'my-calibre', displayName: 'Calibre-Web', catalogUrl: 'https://…/opds',
 *     auth: { username, password } },
 * ]);
 * const { books, errors } = await loadLibrary(registry);
 * ```
 *
 * Reachability depends on the user's server being CORS-open (see the CORS note in
 * `providers/opds.ts`); an unreachable server surfaces in `errors`, never crashing
 * the aggregate.
 */
export function registryWithOpds(configs: readonly OpdsConfig[]): ProviderRegistry {
  const registry = defaultRegistry();
  for (const config of configs) {
    registry.register(createOpdsProvider(config));
  }
  return registry;
}

/** The best on-device index for the current environment. */
export function defaultIndex(): LibraryIndex {
  return idbAvailable() ? new IdbLibraryIndex() : new InMemoryLibraryIndex();
}

export interface LoadLibraryResult {
  books: Book[];
  errors: ProviderRunError[];
}

/**
 * Run the pipeline: aggregate across providers, persist the merged catalog to the
 * index, and return it. Persistence failures are non-fatal — a fresh aggregate is
 * still returned so the UI can render.
 */
export async function loadLibrary(
  registry: ProviderRegistry = defaultRegistry(),
  index: LibraryIndex = defaultIndex(),
): Promise<LoadLibraryResult> {
  const { books, errors } = await aggregateLibrary(registry);

  try {
    await index.put(books);
  } catch {
    // Non-fatal: the catalog is still usable this session even if it could not
    // be cached for offline reuse.
  }

  return { books, errors };
}
