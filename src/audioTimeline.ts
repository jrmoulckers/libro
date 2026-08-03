import type { AudioChapter, PlaybackTrack } from "./types";

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

// ---------------------------------------------------------------------------
// Sleep-timer math (pure; the in-app player timer, not the OS/background one)
// ---------------------------------------------------------------------------

/**
 * Whole seconds remaining until a wall-clock expiry, never negative. Rounded up
 * so the display shows "0:01" for the final partial second rather than jumping
 * straight to 0.
 */
export function sleepRemainingSeconds(expiresAtMs: number, nowMs: number): number {
  return Math.max(0, Math.ceil((expiresAtMs - nowMs) / 1000));
}

/**
 * The book-absolute time at which the chapter containing `absolute` ends — the
 * pause target for an "end of chapter" sleep timer. Returns `null` when there
 * are no chapters or the position is past the last chapter (nothing to end on).
 * Because chapter `end`s are book-absolute (and may span a track boundary), this
 * works across the multi-track timeline.
 */
export function endOfChapterAbsolute(
  chapters: AudioChapter[],
  absolute: number,
): number | null {
  if (!chapters || chapters.length === 0) return null;
  const current = chapters.find((c) => absolute >= c.start && absolute < c.end);
  return current ? current.end : null;
}

/**
 * Volume multiplier for a linear fade-out over the final `fadeSeconds` of a
 * timed sleep countdown: `1` while more than `fadeSeconds` remain, ramping to
 * `0` at expiry. Restored to full on cancel/extend/next play by the caller.
 */
export function fadeMultiplier(remainingSeconds: number, fadeSeconds: number): number {
  if (fadeSeconds <= 0) return 1;
  if (remainingSeconds >= fadeSeconds) return 1;
  if (remainingSeconds <= 0) return 0;
  return remainingSeconds / fadeSeconds;
}
