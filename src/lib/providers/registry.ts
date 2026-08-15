/**
 * Provider registry + library aggregation — the browser analog of the blueprint's
 * `list_all_books`.
 *
 * The registry holds the set of configured connectors; {@link aggregateLibrary}
 * fans out over them, isolates failures, and merges the results into a single
 * deduped {@link Book}[]. This is where a real app wires in OPDS/Audiobookshelf/
 * Libby/Hardcover providers in Phase 2 — nothing else in the pipeline changes.
 */

import { dedupeBooks, type Book } from '../models';
import type { Provider } from './types';

/** Ordered collection of the connectors the app should pull from. */
export class ProviderRegistry {
  readonly #providers = new Map<string, Provider>();

  constructor(providers: readonly Provider[] = []) {
    for (const provider of providers) {
      this.register(provider);
    }
  }

  /** Register (or replace) a provider by its {@link Provider.id}. */
  register(provider: Provider): this {
    this.#providers.set(provider.id, provider);
    return this;
  }

  /** Remove a provider by id. Returns whether one was removed. */
  unregister(id: string): boolean {
    return this.#providers.delete(id);
  }

  /** All registered providers, in registration order. */
  list(): Provider[] {
    return [...this.#providers.values()];
  }
}

/** Outcome for a single provider during an aggregation run. */
export interface ProviderRunError {
  providerId: string;
  error: unknown;
}

/** Result of {@link aggregateLibrary}: the merged catalog plus any failures. */
export interface AggregateResult {
  /** Deduped, merged catalog across every provider that succeeded. */
  books: Book[];
  /** Providers that threw; the aggregate still succeeds without them. */
  errors: ProviderRunError[];
}

/**
 * Fan out over every provider's `listBooks`, isolate failures, and merge into a
 * single deduped catalog.
 *
 * Failure isolation: one provider throwing (network error, bad auth, malformed
 * response) must never abort the whole aggregate. We use `Promise.allSettled`,
 * collect rejected providers into {@link AggregateResult.errors}, and merge only
 * the fulfilled results. Callers can surface partial failures without losing the
 * providers that did work.
 */
export async function aggregateLibrary(
  source: ProviderRegistry | readonly Provider[],
): Promise<AggregateResult> {
  const providers = source instanceof ProviderRegistry ? source.list() : [...source];

  const settled = await Promise.allSettled(providers.map((p) => p.listBooks()));

  const books: Book[] = [];
  const errors: ProviderRunError[] = [];

  settled.forEach((result, index) => {
    const provider = providers[index];
    if (!provider) return;
    if (result.status === 'fulfilled') {
      books.push(...result.value);
    } else {
      errors.push({ providerId: provider.id, error: result.reason });
    }
  });

  return { books: dedupeBooks(books), errors };
}
