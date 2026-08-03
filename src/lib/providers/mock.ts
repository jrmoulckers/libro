/**
 * In-repo mock provider.
 *
 * Returns a handful of fixture {@link Book}s so the full pipeline
 * (registry -> aggregation -> index -> UI) can be exercised end-to-end with zero
 * network and no CORS. It stands in until real connectors (OPDS, Audiobookshelf,
 * Libby, Hardcover) land in Phase 2, at which point it can stay as a dev/demo
 * source or be dropped from the registry.
 *
 * It advertises only the `catalog` capability — it can enumerate a library and
 * nothing more.
 */

import type { Book } from '../models';
import type { Provider, ProviderCapability } from './types';

const MOCK_PROVIDER_ID = 'mock';

const FIXTURES: readonly Omit<Book, 'sourceProviderId'>[] = [
  {
    id: 'piranesi',
    title: 'Piranesi',
    authors: ['Susanna Clarke'],
    mediaType: 'ebook',
    identifiers: { isbn: '9781635575637' },
    description: 'A solitary man explores an endless labyrinth of halls and tides.',
    progress: { fraction: 0.42, locator: 'epubcfi(/6/14!/4/2)', finished: false },
  },
  {
    id: 'babel',
    title: 'Babel',
    authors: ['R. F. Kuang'],
    series: 'Babel',
    mediaType: 'audiobook',
    identifiers: { isbn: '9780063021426', asin: 'B09TQR6M4V' },
    progress: { fraction: 1, positionSeconds: 0, finished: true },
  },
  {
    id: 'ancillary-justice',
    title: 'Ancillary Justice',
    authors: ['Ann Leckie'],
    series: 'Imperial Radch',
    mediaType: 'ebook',
    identifiers: { isbn: '9780356502403' },
  },
  {
    id: 'the-rest-is-history',
    title: 'The Rest Is History',
    authors: ['Tom Holland', 'Dominic Sandbrook'],
    mediaType: 'podcast',
  },
];

/**
 * Create a mock provider. Pass fixtures to override the defaults (handy in tests);
 * every returned book is stamped with this provider's id as its source.
 */
export function createMockProvider(
  fixtures: readonly Omit<Book, 'sourceProviderId'>[] = FIXTURES,
): Provider {
  const capabilities: ReadonlySet<ProviderCapability> = new Set(['catalog']);

  return {
    id: MOCK_PROVIDER_ID,
    displayName: 'Mock Library',
    capabilities,
    async listBooks(): Promise<Book[]> {
      return fixtures.map((fixture) => ({
        ...fixture,
        authors: [...fixture.authors],
        sourceProviderId: MOCK_PROVIDER_ID,
      }));
    },
  };
}

export { MOCK_PROVIDER_ID };
