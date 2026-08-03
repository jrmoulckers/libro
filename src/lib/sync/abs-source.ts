/**
 * The Audiobookshelf (ABS) {@link ProgressSource}: pull the current listening
 * position from a user's `mediaProgress`, and push a device-local position back.
 *
 * ## Endpoints (real ABS API)
 *  - Pull: `GET {baseUrl}/api/me` → `mediaProgress[]`, matched to the book by the
 *    `abs:item_id` we stored in `Book.identifiers` (the ABS `libraryItemId`).
 *  - Push: `PATCH {baseUrl}/api/me/progress/{libraryItemId}` with
 *    `{ currentTime, duration, progress, isFinished }`.
 *
 * Auth is a **Bearer** API token (`Authorization: Bearer …`), same as the ABS
 * catalog connector. Unlike a cover in an `<img>`, these are `fetch` calls, so the
 * header works — the only caveat is **CORS**: the user's ABS server must send
 * permissive `Access-Control-Allow-Origin` for the browser to read the response
 * (see the connector's CORS note). The JSON→{@link RemoteProgress} mapping lives in
 * the pure, unit-tested {@link mapAbsMediaProgress}; only the `fetch` is the shell.
 */

import type { Book, Progress } from '../models';
import type { AbsConfig } from '../providers/audiobookshelf';
import type { RemoteProgress } from './reconcile';
import type { ProgressSource } from './source';
import { SyncError } from './source';

/** The `abs:item_id` identifier we persisted on ABS-sourced books. */
export const ABS_ITEM_ID_KEY = 'abs:item_id';

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(Math.max(value, 0), 1);
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

/**
 * Map an ABS `/api/me` payload to this book's {@link RemoteProgress}, or `null`
 * when the user has no progress record for it. Pure — feed it fixture JSON.
 *
 * Matches the `mediaProgress[]` entry whose `libraryItemId` equals `absItemId`
 * (book-level entries only; podcast `episodeId` rows are ignored). `progress` is
 * ABS's own `0..1` fraction; `currentTime` the position in seconds; `lastUpdate`
 * is epoch **milliseconds** on the wire and is converted to **seconds** for the
 * newest-wins branch.
 */
export function mapAbsMediaProgress(meJson: unknown, absItemId: string): RemoteProgress | null {
  const record = asRecord(meJson);
  const list = record?.mediaProgress;
  if (!Array.isArray(list)) return null;

  for (const raw of list) {
    const entry = asRecord(raw);
    if (!entry) continue;
    if (entry.libraryItemId !== absItemId) continue;
    // Skip podcast-episode rows; a book's progress row has no episodeId.
    if (entry.episodeId != null && entry.episodeId !== '') continue;

    const fraction = clamp01(asNumber(entry.progress) ?? 0);
    const finished = entry.isFinished === true || fraction >= 0.99;
    const lastUpdateMs = asNumber(entry.lastUpdate);
    const remote: RemoteProgress = {
      fraction,
      finished,
      updatedAt: lastUpdateMs !== undefined ? Math.floor(lastUpdateMs / 1000) : null,
    };
    const currentTime = asNumber(entry.currentTime);
    if (currentTime !== undefined) remote.positionSeconds = currentTime;
    return remote;
  }
  return null;
}

function normalizeBaseUrl(url: string): string {
  return url.replace(/\/+$/, '');
}

function absItemId(book: Book): string | undefined {
  const id = book.identifiers?.[ABS_ITEM_ID_KEY];
  return id && id.length > 0 ? id : undefined;
}

/**
 * Construct an ABS {@link ProgressSource} from on-device connector config. Holds
 * no secrets beyond the user-supplied API token already in {@link AbsConfig};
 * nothing is logged or sent anywhere but the user's own server.
 */
export function createAbsProgressSource(config: AbsConfig): ProgressSource {
  const base = normalizeBaseUrl(config.baseUrl);
  const authHeaders = { Authorization: `Bearer ${config.apiToken}` };

  return {
    id: config.id,
    displayName: config.displayName,

    async pullProgress(book: Book): Promise<RemoteProgress | null> {
      const itemId = absItemId(book);
      if (!itemId) return null;

      let response: Response;
      try {
        response = await fetch(`${base}/api/me`, {
          headers: { Accept: 'application/json', ...authHeaders },
        });
      } catch (cause) {
        throw new SyncError(`Audiobookshelf /api/me request failed`, cause);
      }
      if (!response.ok) {
        throw new SyncError(`Audiobookshelf /api/me returned ${response.status}`);
      }
      let payload: unknown;
      try {
        payload = await response.json();
      } catch (cause) {
        throw new SyncError(`Audiobookshelf /api/me returned malformed JSON`, cause);
      }
      return mapAbsMediaProgress(payload, itemId);
    },

    async pushProgress(book: Book, progress: Progress): Promise<void> {
      const itemId = absItemId(book);
      if (!itemId) return;

      const body: Record<string, unknown> = {
        progress: clamp01(progress.fraction),
        isFinished: progress.finished,
      };
      if (progress.positionSeconds !== undefined) body.currentTime = progress.positionSeconds;

      let response: Response;
      try {
        response = await fetch(`${base}/api/me/progress/${encodeURIComponent(itemId)}`, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json', ...authHeaders },
          body: JSON.stringify(body),
        });
      } catch (cause) {
        throw new SyncError(`Audiobookshelf progress push failed`, cause);
      }
      if (!response.ok) {
        throw new SyncError(`Audiobookshelf progress push returned ${response.status}`);
      }
    },
  };
}
