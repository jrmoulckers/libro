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
import { createAbsProvider, type AbsConfig } from './providers/audiobookshelf';
import { aggregateLibrary, ProviderRegistry } from './providers/registry';
import type { ProviderRunError } from './providers/registry';
import { createLocalProvider } from './local/import';
import { IdbLocalStore, InMemoryLocalStore, type LocalStore } from './local/store';
import { IdbReadingStore, InMemoryReadingStore, type ReadingStore } from './reader/reading-store';
import {
  IdbListeningStore,
  InMemoryListeningStore,
  type ListeningStore,
} from './player/listening-store';
import { createSamplePlaybackSource } from './player/sample-source';
import type { PlaybackSource } from './player/source';
import type { EnrichResult } from './metadata/enrich';

/**
 * The providers the app pulls from. The mock provider is the only *default*
 * source; real connectors are registered on top of this via the config-driven
 * helpers ({@link registryWithOpds}/{@link registryWithAbs}) or, for the local
 * file provider, via {@link appRegistry}. Kept mock-only so unit tests and the
 * no-configured-sources path have a deterministic, secret-free baseline.
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

/**
 * Config-driven registration of the real Audiobookshelf connector, kept separate
 * from {@link defaultRegistry} so no live API tokens are baked into the default
 * pipeline. A UI/settings layer collects {@link AbsConfig}s from on-device storage
 * (there is no server to hold secrets) and calls this:
 *
 * ```ts
 * const registry = registryWithAbs([
 *   { id: 'home-abs', displayName: 'Audiobookshelf', baseUrl: 'https://abs.example.com',
 *     apiToken: token, libraryId },
 * ]);
 * const { books, errors } = await loadLibrary(registry);
 * ```
 *
 * As with OPDS, reachability depends on the user's ABS server being CORS-open (see
 * the CORS note in `providers/audiobookshelf.ts`); an unreachable server surfaces
 * in `errors`, never crashing the aggregate.
 */
export function registryWithAbs(configs: readonly AbsConfig[]): ProviderRegistry {
  const registry = defaultRegistry();
  for (const config of configs) {
    registry.register(createAbsProvider(config));
  }
  return registry;
}

/** The best on-device index for the current environment. */
export function defaultIndex(): LibraryIndex {
  return idbAvailable() ? new IdbLibraryIndex() : new InMemoryLibraryIndex();
}

/** The best on-device store for imported local EPUBs in the current environment. */
export function defaultLocalStore(): LocalStore {
  return idbAvailable() ? new IdbLocalStore() : new InMemoryLocalStore();
}

/**
 * The best on-device store for per-book reading positions in the current
 * environment. Lives in its own `libro-reading` database (see
 * {@link IdbReadingStore}); P8 progress-sync reconciles it with a remote tracker.
 */
export function defaultReadingStore(): ReadingStore {
  return idbAvailable() ? new IdbReadingStore() : new InMemoryReadingStore();
}

/**
 * The best on-device store for per-book **listening** positions (book-absolute
 * seconds over the multi-track audio timeline). Its own `libro-listening`
 * database (see {@link IdbListeningStore}); P8 progress-sync reconciles it with a
 * remote tracker (Audiobookshelf `PATCH /api/me/progress/{id}`).
 */
export function defaultListeningStore(): ListeningStore {
  return idbAvailable() ? new IdbListeningStore() : new InMemoryListeningStore();
}

/**
 * The playback source the running app uses to *play* audiobooks. Kept mock-only
 * alongside {@link defaultRegistry}: the bundled sample source synthesizes a
 * multi-track demo audiobook (no network, no committed assets) so the player is
 * exercisable. A real Audiobookshelf audiobook would instead be resolved by
 * `createAbsPlaybackSource(absConfig)` (see `player/abs-source.ts`), selected by
 * matching `book.sourceProviderId` to the configured ABS provider id — the audio
 * analog of {@link registryWithAbs}. That opt-in wiring lands with the settings
 * UI; no secrets are baked in here.
 */
export function defaultPlaybackSource(): PlaybackSource {
  return createSamplePlaybackSource();
}

/**
 * The registry the running app uses: the mock baseline **plus** the local-file
 * provider, which is safe to include unconditionally — it reads only from the
 * on-device {@link LocalStore} (no network, no secrets), so it never introduces a
 * CORS/credential concern. Real remote connectors stay opt-in via
 * {@link registryWithOpds}/{@link registryWithAbs}; that is why the local
 * provider joins here rather than in {@link defaultRegistry} (which must stay a
 * pure, secret-free unit-test baseline).
 *
 * The caller passes the same {@link LocalStore} instance it uses for imports so
 * the two share one on-device database.
 */
export function appRegistry(store: LocalStore = defaultLocalStore()): ProviderRegistry {
  const registry = defaultRegistry();
  registry.register(createLocalProvider(store));
  return registry;
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

/**
 * Fill gaps (cover, description, authors, series, subjects) on an already-loaded
 * catalog from the **CORS-open** public metadata APIs, then re-persist the
 * enriched catalog to the index.
 *
 * This is deliberately a **separate, post-render** step from {@link loadLibrary}:
 * the app renders the aggregated library first and calls this in the background
 * so enrichment never delays first paint. Unlike the user-server connectors
 * (OPDS/ABS), Open Library and Google Books send permissive CORS headers, so
 * these fetches genuinely work in the browser with no app-owned proxy — this is
 * the one place Libro does real live network enrichment. Results are cached by
 * ISBN (see {@link ./metadata/cache}) so repeat launches are free and offline.
 *
 * Failure-isolated end to end: a failing lookup surfaces in `errors` and never
 * throws; a persistence failure is swallowed (the enriched catalog is still shown
 * this session).
 */
export async function enrichLibrary(
  books: readonly Book[],
  index: LibraryIndex = defaultIndex(),
): Promise<EnrichResult> {
  // Lazy-load the metadata layer so its parsers/fetchers/cache stay OUT of the main
  // entry chunk — enrichment runs post-paint, so its code can arrive on demand too.
  const { enrichBooks, liveEnrichDeps } = await import('./metadata/enrich');
  const result = await enrichBooks(books, liveEnrichDeps());
  try {
    await index.put(result.books);
  } catch {
    // Non-fatal: enrichment still applies in-memory this session.
  }
  return result;
}
