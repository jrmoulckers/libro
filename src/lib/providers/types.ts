/**
 * The connector/plugin contract — a first-class abstraction in Libro.
 *
 * A {@link Provider} is Libro's contract for talking to one external source of
 * books/audiobooks (Audiobookshelf, a public library via OverDrive/Libby,
 * Hardcover, Open Library, a local folder, …). Everything the app can do with a
 * backend is expressed through this interface, which lets the aggregation layer
 * treat every source uniformly and makes adding a connector a matter of
 * implementing one interface.
 *
 * This is a **pure-client** design: a `Provider` implementation talks *directly*
 * to the remote API from the user's device via `fetch`. There is no Libro server
 * in the middle — do not add one.
 */

import type { Book } from '../models';

/**
 * The set of features a connector supports.
 *
 * Mirrors the blueprint's capability bitflags as a string-literal union so the UI
 * can enable/disable actions per provider without probing. Providers advertise
 * exactly what they can do; the aggregation layer never assumes a capability is
 * present.
 */
export type ProviderCapability =
  /** Can enumerate the user's library ({@link Provider.listBooks}). */
  | 'catalog'
  /** Can place/track holds (e.g. library systems). */
  | 'holds'
  /** Can request/acquire a title not yet owned. */
  | 'request'
  /** Can download the underlying file(s). */
  | 'download'
  /** Can push a title to a Kindle (typically via Send-to-Kindle email). */
  | 'send-to-kindle'
  /** Can read and/or write reading/listening progress. */
  | 'progress-sync'
  /**
   * Not a real integration: surfaced as an "open in the official app" deep link
   * (e.g. Libby/OverDrive). Connectors with this flag must never call an API.
   */
  | 'deep-link-only';

/** Every capability, useful for validation and tests. */
export const PROVIDER_CAPABILITIES: readonly ProviderCapability[] = [
  'catalog',
  'holds',
  'request',
  'download',
  'send-to-kindle',
  'progress-sync',
  'deep-link-only',
];

/**
 * The connector contract. Implement this to add a new source to Libro.
 *
 * `listBooks` is async because real connectors perform network I/O.
 *
 * ## Phase 2+ plug-in points (intentionally not part of the foundation)
 * Later phases extend this contract; connectors declare the matching capability:
 *  - `authenticate(config)` — verify credentials before listing (uses config from
 *    on-device storage; there is no server to hold secrets).
 *  - `placeHold` / `request` — for `holds` / `request` capable library systems.
 *  - `download` / `sendToKindle` — for `download` / `send-to-kindle`.
 *  - `getProgress` / `setProgress` — for `progress-sync`, consumed by the reader
 *    and player.
 * Keep those off the interface until their phase lands so the foundation stays
 * minimal and every current implementation stays trivially satisfiable.
 */
export interface Provider {
  /**
   * Stable, machine-readable identifier for this connector *type*
   * (e.g. `"audiobookshelf"`). Used as {@link Book.sourceProviderId}.
   */
  readonly id: string;

  /** Human-friendly name for display in the UI (e.g. `"Audiobookshelf"`). */
  readonly displayName: string;

  /** What this connector can do. See {@link ProviderCapability}. */
  readonly capabilities: ReadonlySet<ProviderCapability>;

  /**
   * Enumerate the user's library as normalized {@link Book}s.
   *
   * Requires the `catalog` capability. Implementations should map their native
   * response onto `Book` and set `sourceProviderId` to {@link Provider.id}.
   */
  listBooks(): Promise<Book[]>;
}
