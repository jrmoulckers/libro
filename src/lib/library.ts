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
import { aggregateLibrary, ProviderRegistry } from './providers/registry';
import type { ProviderRunError } from './providers/registry';

/**
 * The providers the app pulls from. Phase 2 registers real connectors here; the
 * rest of the pipeline is untouched.
 */
export function defaultRegistry(): ProviderRegistry {
  return new ProviderRegistry([createMockProvider()]);
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
