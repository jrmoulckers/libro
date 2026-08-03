import type { AudioChapter, Book, PlaybackTrack } from "./types";

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

// ---------------------------------------------------------------------------
// Chapter navigation (pure) — powers previoustrack/nexttrack media-session keys
// ---------------------------------------------------------------------------

/**
 * The index of the chapter containing `absolute`, or `-1` when there are no
 * chapters / the position is outside every chapter range. Book-absolute, so it
 * spans track boundaries.
 */
function currentChapterIndex(chapters: AudioChapter[], absolute: number): number {
  if (!chapters || chapters.length === 0) return -1;
  return chapters.findIndex((c) => absolute >= c.start && absolute < c.end);
}

/** How far into the current chapter (seconds) we still treat "previous" as a
 * restart of the current chapter rather than a jump to the prior one. */
const PREV_CHAPTER_RESTART_THRESHOLD = 3;

/**
 * The book-absolute start of the chapter the "next chapter" transport should
 * jump to, or `null` when there is no next chapter (no chapters, or already in
 * the last one) so the caller can fall back to a plain skip-forward.
 */
export function nextChapterStart(
  chapters: AudioChapter[],
  absolute: number,
): number | null {
  const idx = currentChapterIndex(chapters, absolute);
  if (idx < 0) return null;
  const next = chapters[idx + 1];
  return next ? next.start : null;
}

/**
 * The book-absolute target for the "previous chapter" transport. Mirrors the
 * familiar media behavior: if we're more than a few seconds into the current
 * chapter, restart it; otherwise jump to the previous chapter's start. Returns
 * `null` only when there are no chapters (caller falls back to skip-backward).
 */
export function prevChapterStart(
  chapters: AudioChapter[],
  absolute: number,
): number | null {
  const idx = currentChapterIndex(chapters, absolute);
  if (idx < 0) return null;
  const current = chapters[idx];
  if (absolute - current.start > PREV_CHAPTER_RESTART_THRESHOLD) {
    return current.start;
  }
  const prev = chapters[idx - 1];
  return prev ? prev.start : current.start;
}

// ---------------------------------------------------------------------------
// Media Session inputs (pure) — OS now-playing / lockscreen / media keys
// ---------------------------------------------------------------------------

/** The plain inputs for a `MediaMetadata` (built into the DOM object by the
 * caller, which owns the browser API). */
export interface MediaMetadataInput {
  title: string;
  artist: string;
  album: string;
  artwork: { src: string }[];
}

/** The plain inputs for `MediaSession.setPositionState`. */
export interface PositionStateInput {
  duration: number;
  position: number;
  playbackRate: number;
}

/**
 * A cover URL is only usable in the OS now-playing card if the OS can actually
 * fetch it: `http(s)` and `data:` URLs qualify. Our embedded-cover scheme
 * `localcover://{id}` is a Tauri-internal reference the OS can't resolve, so it
 * is dropped (documented caveat) — the card then shows no artwork rather than a
 * broken image.
 */
function usableArtwork(coverUrl: string | null | undefined): { src: string }[] {
  if (!coverUrl) return [];
  if (/^(https?:|data:)/i.test(coverUrl)) return [{ src: coverUrl }];
  return [];
}

/**
 * Build the OS now-playing metadata from the current book (falling back to the
 * player's title). Pure and DOM-free so it can be unit-tested; the caller wraps
 * the result in `new MediaMetadata(...)`.
 */
export function mediaMetadataInput(
  book: Book | undefined,
  fallbackTitle: string | undefined,
): MediaMetadataInput {
  const title = book?.title ?? fallbackTitle ?? "Now playing";
  const artist =
    book?.authors && book.authors.length > 0
      ? book.authors.join(", ")
      : "Unknown author";
  // Album = the book title (the "work"); series is a nice sub-label when present.
  const album = book?.series ?? book?.title ?? title;
  return { title, artist, album, artwork: usableArtwork(book?.cover_url) };
}

/**
 * Build the `setPositionState` payload on the **book-absolute unified timeline**
 * so the OS scrubber reflects whole-book progress, not the current file.
 * Returns `null` when the duration is not yet known/valid (the spec throws a
 * `TypeError` for a non-positive duration or a position past it), signalling the
 * caller to skip the call this tick.
 */
export function positionStateInput(
  total: number,
  absolute: number,
  playbackRate: number,
): PositionStateInput | null {
  if (!Number.isFinite(total) || total <= 0) return null;
  const duration = total;
  const position = Math.max(0, Math.min(absolute, duration));
  const rate = Number.isFinite(playbackRate) && playbackRate > 0 ? playbackRate : 1;
  return { duration, position, playbackRate: rate };
}
