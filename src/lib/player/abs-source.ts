/**
 * Audiobookshelf (ABS) playback source — resolves an ABS audiobook to a directly
 * playable multi-track {@link PlaybackManifest}.
 *
 * Uses `POST {baseUrl}/api/items/{itemId}/play`, which opens a playback session
 * and returns the item's audio tracks + chapter markers. The heavy lifting — URL
 * resolution, book-absolute timeline layout, chapter mapping — is the pure,
 * unit-tested {@link mapAbsPlaybackSession}; `createAbsPlaybackSource` is just the
 * authenticated fetch around it.
 *
 * ## Auth + the `<audio>` token caveat (mirrors the cover caveat)
 * ABS requests authenticate with the per-user Bearer token, but an HTML
 * `<audio src>` element cannot send an `Authorization` header. So each track's
 * stream URL carries the token as a `?token=` query param instead
 * ({@link resolveStreamUrl}) — the same workaround the connector uses for covers.
 *
 * ## Browser constraint — CORS + range requests
 * Like every remote source, this only works if the user's ABS server is CORS-open
 * (studio rule: no app-owned proxy). Seeking additionally relies on the server
 * honoring HTTP range requests (ABS does). A failing fetch throws {@link AbsPlaybackError}
 * so the caller can surface it without crashing the app.
 *
 * ## Phase 8 TODO
 * `PATCH /api/me/progress/{itemId}` push/pull of the listening position is
 * deferred to P8 (progress sync); this phase only resolves the playable manifest.
 */

import { AbsError, type AbsConfig } from '../providers/audiobookshelf';
import type { Book } from '../models';
import type { Chapter, PlaybackManifest, PlaybackTrack } from './timeline';
import { assembleTimeline } from './timeline';
import type { PlaybackSource } from './source';

/** Typed error thrown by ABS playback-session fetch/parse failures. */
export class AbsPlaybackError extends Error {
  constructor(
    message: string,
    override readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'AbsPlaybackError';
  }
}

/**
 * Resolve an ABS track `contentUrl` (absolute or server-relative) to an absolute
 * URL, appending the auth token as a `?token=` query param when present. Pure.
 */
export function resolveStreamUrl(baseUrl: string, contentUrl: string, token: string): string {
  const base = baseUrl.replace(/\/+$/, '');
  let absolute: string;
  if (/^https?:\/\//i.test(contentUrl)) {
    absolute = contentUrl;
  } else if (contentUrl.startsWith('/')) {
    absolute = `${base}/${contentUrl.slice(1)}`;
  } else {
    absolute = `${base}/${contentUrl}`;
  }
  if (!token) return absolute;
  const sep = absolute.includes('?') ? '&' : '?';
  return `${absolute}${sep}token=${encodeURIComponent(token)}`;
}

/**
 * Map an ABS `/play` session payload into a playable {@link PlaybackManifest}:
 * resolve each audio track's URL (token in the query), lay them on the
 * book-absolute timeline, and normalize chapters. Pure: no network.
 *
 * @throws {AbsPlaybackError} when the session has no audio tracks.
 */
export function mapAbsPlaybackSession(
  session: unknown,
  baseUrl: string,
  token: string,
): PlaybackManifest {
  const record = asRecord(session) ?? {};
  const rawTracks =
    asArray(record.audioTracks) ?? asArray(record.tracks) ?? asArray(record.audioFiles) ?? [];
  if (rawTracks.length === 0) {
    throw new AbsPlaybackError('ABS play session has no audio tracks');
  }

  const tracks: PlaybackTrack[] = rawTracks.map((raw) => {
    const t = asRecord(raw) ?? {};
    const contentUrl = asString(t.contentUrl) ?? asString(t.url) ?? '';
    return {
      url: resolveStreamUrl(baseUrl, contentUrl, token),
      durationSeconds: asNumber(t.duration) ?? 0,
      mimeType: asString(t.mimeType),
    };
  });

  const chapters: Chapter[] = (asArray(record.chapters) ?? []).flatMap((raw, i) => {
    const c = asRecord(raw);
    if (!c) return [];
    const start = asNumber(c.start);
    const end = asNumber(c.end);
    if (start == null || end == null) return [];
    return [
      { title: asString(c.title) ?? `Chapter ${i + 1}`, startAbsolute: start, endAbsolute: end },
    ];
  });

  // Reuse the pure timeline layout so a manifest's tracks are the canonical shape.
  const timeline = assembleTimeline(tracks, chapters);
  return { tracks: timeline.tracks, chapters: timeline.chapters };
}

/**
 * Create an ABS playback source from the same {@link AbsConfig} the catalog
 * connector uses. `resolve(book)` reads the ABS item id from
 * `identifiers['abs:item_id']` and opens a play session for it.
 */
export function createAbsPlaybackSource(config: AbsConfig): PlaybackSource {
  const base = config.baseUrl.replace(/\/+$/, '');
  return {
    id: config.id,
    async resolve(book: Book): Promise<PlaybackManifest> {
      const itemId = book.identifiers?.['abs:item_id'];
      if (!itemId) {
        throw new AbsPlaybackError(`book ${book.id} has no abs:item_id to play`);
      }
      const url = `${base}/api/items/${encodeURIComponent(itemId)}/play`;
      let response: Response;
      try {
        response = await fetch(url, {
          method: 'POST',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            Authorization: `Bearer ${config.apiToken}`,
          },
          body: '{}',
        });
      } catch (cause) {
        throw new AbsPlaybackError(`Audiobookshelf play request to ${url} failed`, cause);
      }
      if (!response.ok) {
        throw new AbsPlaybackError(`Audiobookshelf play request returned ${response.status}`);
      }
      let payload: unknown;
      try {
        payload = await response.json();
      } catch (cause) {
        throw new AbsPlaybackError('Audiobookshelf returned malformed play session JSON', cause);
      }
      return mapAbsPlaybackSession(payload, base, config.apiToken);
    },
  };
}

// AbsError is re-exported so callers can catch either catalog or playback failures
// from a single import when wiring an ABS server end-to-end.
export { AbsError };

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

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}
