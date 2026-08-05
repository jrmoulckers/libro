/**
 * Pure reading-position math + serialization for the reader.
 *
 * A reading position is a lightweight locator: which spine document
 * (`spineIndex`) and how far down it the reader is scrolled (`scrollFraction`).
 * That is deliberately coarser than a full EPUB CFI — a proper CFI is a
 * documented TODO — but it is enough to resume the user where they left off and
 * to show a library-wide reading percentage.
 *
 * Everything here is pure and unit-tested: no DOM, no zip, no store.
 */

import type { Progress } from '../models';

/** A persisted per-book reading position. */
export interface ReadingPosition {
  /** Index into the spine of the currently-open document. */
  spineIndex: number;
  /** Scroll offset within that document, `0..1`. */
  scrollFraction: number;
  /** Book-wide completion `0..1`, precomputed via {@link readingFraction}. */
  fraction: number;
  /** Whether the book is effectively finished. */
  finished: boolean;
}

/** Completion at or past which a book counts as finished. */
export const FINISHED_THRESHOLD = 0.999;

/**
 * Book-wide completion fraction from a spine position. Pure.
 *
 * Treats each spine document as an equal slice of the book (a coarse but
 * dependency-free model): a position is `(spineIndex + scrollFraction) /
 * spineCount`, clamped to `0..1`. Returns `0` for an empty/invalid spine.
 */
export function readingFraction(
  spineIndex: number,
  scrollFraction: number,
  spineCount: number,
): number {
  if (spineCount <= 0) return 0;
  const within = clamp01(scrollFraction);
  const index = Math.min(Math.max(spineIndex, 0), spineCount - 1);
  return clamp01((index + within) / spineCount);
}

/**
 * Build a {@link ReadingPosition} for a spine location, computing its book-wide
 * fraction and finished flag. Pure.
 */
export function makePosition(
  spineIndex: number,
  scrollFraction: number,
  spineCount: number,
): ReadingPosition {
  const fraction = readingFraction(spineIndex, scrollFraction, spineCount);
  // "Finished" means the reader has reached the end of the last document.
  const finished = spineIndex >= spineCount - 1 && clamp01(scrollFraction) >= FINISHED_THRESHOLD;
  return { spineIndex, scrollFraction: clamp01(scrollFraction), fraction, finished };
}

/** Serialize a position to an opaque `Progress.locator` string. Pure. */
export function encodeLocator(
  position: Pick<ReadingPosition, 'spineIndex' | 'scrollFraction'>,
): string {
  return JSON.stringify({
    spineIndex: position.spineIndex,
    scrollFraction: position.scrollFraction,
  });
}

/**
 * Parse a locator string produced by {@link encodeLocator}. Returns `undefined`
 * for anything malformed. Pure.
 */
export function decodeLocator(
  locator: string | undefined,
): { spineIndex: number; scrollFraction: number } | undefined {
  if (!locator) return undefined;
  try {
    const parsed: unknown = JSON.parse(locator);
    if (typeof parsed !== 'object' || parsed === null) return undefined;
    const record = parsed as Record<string, unknown>;
    if (typeof record.spineIndex !== 'number' || typeof record.scrollFraction !== 'number') {
      return undefined;
    }
    return { spineIndex: record.spineIndex, scrollFraction: record.scrollFraction };
  } catch {
    return undefined;
  }
}

/** Project a reading position onto the shared {@link Progress} model. Pure. */
export function positionToProgress(position: ReadingPosition): Progress {
  return {
    fraction: position.fraction,
    locator: encodeLocator(position),
    finished: position.finished,
  };
}

function clamp01(value: number): number {
  if (Number.isNaN(value)) return 0;
  return Math.min(Math.max(value, 0), 1);
}
