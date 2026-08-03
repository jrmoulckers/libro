/**
 * Lane routing + the pure translations between the shared {@link Progress} model
 * and each local store's native position shape, plus thin {@link ProgressStore}
 * adapters over the P5 reading store and P6 listening store.
 *
 * **Lanes never cross.** An audiobook is reconciled through the *listening* lane
 * (seconds ↔ Audiobookshelf); an ebook through the *reading* lane (locator ↔ a
 * text tracker). {@link laneForBook} decides a book's lane from its media type;
 * the reconcile policy itself is lane-agnostic.
 *
 * The conversions ({@link progressToReadingPosition} etc.) are pure and unit-
 * tested; only the store `get`/`put` I/O is the thin, faked-in-tests shell.
 */

import type { Book, Progress } from '../models';
import type { ReadingStore } from '../reader/reading-store';
import type { ReadingPosition } from '../reader/locator';
import { decodeLocator, encodeLocator } from '../reader/locator';
import type { ListeningStore, ListeningPosition } from '../player/listening-store';
import type { LocalProgress, ProgressStore } from './source';

/** Which reconciliation lane a book belongs to. */
export type SyncLane = 'reading' | 'listening';

/**
 * Decide a book's lane from its media type, or `null` when it isn't reconcilable.
 * Audiobooks use the listening lane; ebooks the reading lane; anything else
 * (e.g. podcasts) is skipped. Pure.
 */
export function laneForBook(book: Book): SyncLane | null {
  if (book.mediaType === 'audiobook') return 'listening';
  if (book.mediaType === 'ebook') return 'reading';
  return null;
}

function clamp01(value: number): number {
  if (Number.isNaN(value)) return 0;
  return Math.min(Math.max(value, 0), 1);
}

// ---- reading lane conversions (pure) ------------------------------------

/** Project a stored {@link ReadingPosition} onto the shared {@link Progress}. Pure. */
export function readingPositionToProgress(position: ReadingPosition): Progress {
  return {
    fraction: position.fraction,
    locator: encodeLocator(position),
    finished: position.finished,
  };
}

/**
 * Build a {@link ReadingPosition} from a reconciled {@link Progress}. Recovers the
 * precise spine location from the progress locator when present; otherwise falls
 * back to a coarse position derived from the fraction (spine index 0). Pure.
 */
export function progressToReadingPosition(progress: Progress): ReadingPosition {
  const decoded = decodeLocator(progress.locator);
  return {
    spineIndex: decoded?.spineIndex ?? 0,
    scrollFraction: decoded ? clamp01(decoded.scrollFraction) : clamp01(progress.fraction),
    fraction: clamp01(progress.fraction),
    finished: progress.finished,
  };
}

// ---- listening lane conversions (pure) ----------------------------------

/** Project a stored {@link ListeningPosition} onto the shared {@link Progress}. Pure. */
export function listeningPositionToProgress(position: ListeningPosition): Progress {
  return {
    fraction: position.fraction,
    positionSeconds: position.positionSeconds,
    finished: position.finished,
  };
}

/** Build a {@link ListeningPosition} from a reconciled {@link Progress}. Pure. */
export function progressToListeningPosition(progress: Progress): ListeningPosition {
  return {
    positionSeconds: progress.positionSeconds ?? 0,
    fraction: clamp01(progress.fraction),
    finished: progress.finished,
  };
}

// ---- store adapters (thin I/O shells) -----------------------------------

/** A {@link ProgressStore} view over the P5 {@link ReadingStore}. */
export function readingProgressStore(store: ReadingStore): ProgressStore {
  return {
    async get(bookId: string): Promise<LocalProgress | null> {
      const position = await store.get(bookId);
      // Our local stores carry no per-write timestamp yet → updatedAt null.
      return position ? { progress: readingPositionToProgress(position), updatedAt: null } : null;
    },
    async put(bookId: string, progress: Progress): Promise<void> {
      await store.set(bookId, progressToReadingPosition(progress));
    },
  };
}

/** A {@link ProgressStore} view over the P6 {@link ListeningStore}. */
export function listeningProgressStore(store: ListeningStore): ProgressStore {
  return {
    async get(bookId: string): Promise<LocalProgress | null> {
      const position = await store.get(bookId);
      return position ? { progress: listeningPositionToProgress(position), updatedAt: null } : null;
    },
    async put(bookId: string, progress: Progress): Promise<void> {
      await store.set(bookId, progressToListeningPosition(progress));
    },
  };
}
