import type { PlaybackTrack } from "./types";

/**
 * Pure book-absolute ↔ (track, offset) timeline math for the multi-track audio
 * player. Kept free of React/DOM so it can be reasoned about (and, mirrored by
 * the Rust `assemble_timeline` helper, unit-tested) in isolation. The player
 * treats a multi-file ABS audiobook as ONE continuous timeline; these helpers
 * translate between that unified position and the concrete track the `<audio>`
 * element is actually playing.
 */

/** Total book duration = the sum of every track's (clamped) duration. */
export function totalDuration(tracks: PlaybackTrack[]): number {
  return tracks.reduce(
    (sum, t) => sum + (t.duration_seconds > 0 ? t.duration_seconds : 0),
    0,
  );
}

/**
 * Map a book-absolute position (seconds) to the track that contains it and the
 * offset within that track. The absolute value is clamped to
 * `[0, totalDuration]`, and a position that lands exactly on (or past) the end
 * resolves to the last track so end-of-book seeks don't fall off the list.
 */
export function locateAbsolute(
  tracks: PlaybackTrack[],
  absolute: number,
): { index: number; offset: number } {
  if (tracks.length === 0) return { index: 0, offset: 0 };
  const total = totalDuration(tracks);
  const clamped = Math.max(0, Math.min(absolute, total));
  // Walk from the end so a boundary value (== a later track's start) lands at
  // the START of that later track, not the end of the earlier one.
  for (let i = tracks.length - 1; i >= 0; i--) {
    if (clamped >= tracks[i].start_offset_seconds || i === 0) {
      return { index: i, offset: clamped - tracks[i].start_offset_seconds };
    }
  }
  return { index: 0, offset: 0 };
}

/** The book-absolute position for a given track index + within-track time. */
export function toAbsolute(
  tracks: PlaybackTrack[],
  index: number,
  within: number,
): number {
  const t = tracks[index];
  return (t ? t.start_offset_seconds : 0) + Math.max(0, within);
}
