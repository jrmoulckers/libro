/**
 * Pure book-absolute ↔ (track, offset) timeline math for the audiobook player.
 *
 * An audiobook is MULTIPLE audio files (tracks) whose chapters don't align to
 * file boundaries. The player presents ONE unified **book-absolute** timeline
 * over the concatenated tracks; these helpers translate between that whole-book
 * position and the concrete track the `<audio>` element is actually playing, and
 * compute the sleep-timer / Media-Session / chapter-navigation inputs.
 *
 * Ported faithfully from the blueprint's `audioTimeline.ts` (which mirrors the
 * Rust `assemble_timeline`). Everything here is DOM-free and network-free so it
 * can be reasoned about and unit-tested in isolation; `Player.svelte` is the thin
 * `<audio>`/Media-Session shell on top.
 */

import type { Book, Progress } from '../models';

/** A single playable track (one source file) before timeline layout. */
export interface PlaybackTrack {
  /** Directly-playable stream URL (auth token in the query string when needed). */
  url: string;
  /** This track's own duration in seconds. */
  durationSeconds: number;
  /** MIME type hint (e.g. `audio/mpeg`), if known. */
  mimeType?: string;
}

/** A chapter marker in **book-absolute** seconds (may span a track boundary). */
export interface Chapter {
  title: string;
  startAbsolute: number;
  endAbsolute: number;
}

/** A source's resolved audiobook: ordered tracks + book-absolute chapters. */
export interface PlaybackManifest {
  tracks: PlaybackTrack[];
  chapters: Chapter[];
}

/** A track laid out on the book-absolute timeline. */
export interface TimelineTrack extends PlaybackTrack {
  index: number;
  /** Cumulative start offset = sum of all prior track durations. */
  startOffset: number;
}

/** The assembled, book-absolute timeline. */
export interface Timeline {
  tracks: TimelineTrack[];
  chapters: Chapter[];
  /** Whole-book duration = the sum of every track's (clamped) duration. */
  totalDuration: number;
}

/** A clamped, finite, non-negative duration (a bad value can't rewind the book). */
function safeDuration(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

/**
 * Lay ordered tracks onto a book-absolute timeline: each track's `startOffset`
 * is the cumulative sum of all prior (clamped) durations, and `totalDuration` is
 * their sum. Pure — the single source of truth for the whole-book position.
 */
export function assembleTimeline(
  tracks: readonly PlaybackTrack[],
  chapters: readonly Chapter[] = [],
): Timeline {
  let offset = 0;
  const out: TimelineTrack[] = [];
  tracks.forEach((track, index) => {
    const durationSeconds = safeDuration(track.durationSeconds);
    out.push({ ...track, durationSeconds, index, startOffset: offset });
    offset += durationSeconds;
  });
  return { tracks: out, chapters: [...chapters], totalDuration: offset };
}

/**
 * Map a book-absolute position to the track containing it + the offset within
 * that track. Clamped to `[0, totalDuration]`; a boundary value resolves to the
 * START of the later track (we walk from the end), and end-of-book resolves to
 * the last track. Pure.
 */
export function locateAbsolute(
  tracks: readonly TimelineTrack[],
  absolute: number,
): { trackIndex: number; offsetInTrack: number } {
  if (tracks.length === 0) return { trackIndex: 0, offsetInTrack: 0 };
  const total = tracks.reduce((sum, t) => sum + t.durationSeconds, 0);
  const clamped = Math.max(0, Math.min(absolute, total));
  for (let i = tracks.length - 1; i >= 0; i--) {
    if (clamped >= tracks[i].startOffset || i === 0) {
      return { trackIndex: i, offsetInTrack: clamped - tracks[i].startOffset };
    }
  }
  return { trackIndex: 0, offsetInTrack: 0 };
}

/** The book-absolute position for a track index + within-track time. Pure. */
export function trackToAbsolute(
  tracks: readonly TimelineTrack[],
  trackIndex: number,
  within: number,
): number {
  const track = tracks[trackIndex];
  return (track ? track.startOffset : 0) + Math.max(0, within);
}

// ---------------------------------------------------------------------------
// Sleep-timer math (pure; the in-app player timer)
// ---------------------------------------------------------------------------

/**
 * Whole seconds until a wall-clock expiry, never negative. Rounded up so the
 * final partial second shows "0:01" rather than jumping straight to 0. Pure.
 */
export function sleepRemainingSeconds(expiresAtMs: number, nowMs: number): number {
  return Math.max(0, Math.ceil((expiresAtMs - nowMs) / 1000));
}

/**
 * The book-absolute time at which the chapter containing `absolute` ends — the
 * pause target for an "end of chapter" sleep timer. `null` when there are no
 * chapters or the position is past the last chapter. Works across track
 * boundaries (chapter times are book-absolute). Pure.
 */
export function endOfChapterAbsolute(
  chapters: readonly Chapter[],
  absolute: number,
): number | null {
  if (chapters.length === 0) return null;
  const current = chapters.find((c) => absolute >= c.startAbsolute && absolute < c.endAbsolute);
  return current ? current.endAbsolute : null;
}

/**
 * Volume multiplier for a linear fade-out over the final `fadeSeconds` of a timed
 * sleep countdown: `1` while more than `fadeSeconds` remain, ramping to `0` at
 * expiry. Pure.
 */
export function fadeMultiplier(remainingSeconds: number, fadeSeconds: number): number {
  if (fadeSeconds <= 0) return 1;
  if (remainingSeconds >= fadeSeconds) return 1;
  if (remainingSeconds <= 0) return 0;
  return remainingSeconds / fadeSeconds;
}

// ---------------------------------------------------------------------------
// Chapter navigation (pure) — powers previoustrack/nexttrack media keys
// ---------------------------------------------------------------------------

/** How far into a chapter "previous" restarts it rather than stepping back. */
const PREV_CHAPTER_RESTART_THRESHOLD = 3;

/** The index of the chapter containing `absolute`, or `-1`. Pure. */
function currentChapterIndex(chapters: readonly Chapter[], absolute: number): number {
  if (chapters.length === 0) return -1;
  return chapters.findIndex((c) => absolute >= c.startAbsolute && absolute < c.endAbsolute);
}

/**
 * The book-absolute start of the chapter "next chapter" should jump to, or `null`
 * when there is no next chapter (caller falls back to skip-forward). Pure.
 */
export function nextChapterStart(chapters: readonly Chapter[], absolute: number): number | null {
  const idx = currentChapterIndex(chapters, absolute);
  if (idx < 0) return null;
  const next = chapters[idx + 1];
  return next ? next.startAbsolute : null;
}

/**
 * The book-absolute target for "previous chapter": restart the current chapter
 * when more than a few seconds in, else step to the prior chapter's start. `null`
 * only when there are no chapters (caller falls back to skip-backward). Pure.
 */
export function prevChapterStart(chapters: readonly Chapter[], absolute: number): number | null {
  const idx = currentChapterIndex(chapters, absolute);
  if (idx < 0) return null;
  const current = chapters[idx];
  if (absolute - current.startAbsolute > PREV_CHAPTER_RESTART_THRESHOLD) {
    return current.startAbsolute;
  }
  const prev = chapters[idx - 1];
  return prev ? prev.startAbsolute : current.startAbsolute;
}

// ---------------------------------------------------------------------------
// Media Session inputs (pure) — OS now-playing / lockscreen / media keys
// ---------------------------------------------------------------------------

/** Plain inputs for a `MediaMetadata` (wrapped in the DOM object by the caller). */
export interface MediaMetadataInput {
  title: string;
  artist: string;
  album: string;
  artwork: { src: string }[];
}

/** Plain inputs for `MediaSession.setPositionState`. */
export interface PositionStateInput {
  duration: number;
  position: number;
  playbackRate: number;
}

/**
 * A cover URL is only usable in the OS now-playing card if the OS can fetch it:
 * `http(s)` and `data:` qualify. Our on-device schemes (`localcover:`/`blob:`)
 * can't be resolved by the OS, so they are dropped (the card shows no artwork
 * rather than a broken reference). Pure.
 */
function usableArtwork(coverUrl: string | undefined): { src: string }[] {
  if (!coverUrl) return [];
  return /^(https?:|data:)/i.test(coverUrl) ? [{ src: coverUrl }] : [];
}

/** Build the OS now-playing metadata from a book. Pure; caller wraps it. */
export function mediaMetadataInput(book: Book): MediaMetadataInput {
  const title = book.title || 'Now playing';
  const artist = book.authors.length > 0 ? book.authors.join(', ') : 'Unknown author';
  // Album = series when present (a nice sub-label), else the work's title.
  const album = book.series ?? book.title ?? title;
  return { title, artist, album, artwork: usableArtwork(book.coverUrl) };
}

/**
 * Build the `setPositionState` payload on the **book-absolute** timeline so the
 * OS scrubber reflects whole-book progress. Returns `null` for an unknown/invalid
 * duration (the spec throws for a non-positive duration or out-of-range position),
 * signalling the caller to skip the call this tick. Pure.
 */
export function positionStateInput(
  total: number,
  absolute: number,
  playbackRate: number,
): PositionStateInput | null {
  if (!Number.isFinite(total) || total <= 0) return null;
  const position = Math.max(0, Math.min(absolute, total));
  const rate = Number.isFinite(playbackRate) && playbackRate > 0 ? playbackRate : 1;
  return { duration: total, position, playbackRate: rate };
}

// ---------------------------------------------------------------------------
// Progress projection (pure)
// ---------------------------------------------------------------------------

/** Book-wide completion `absolute / total`, clamped `0..1`. Pure. */
export function listeningFraction(absolute: number, total: number): number {
  if (!Number.isFinite(total) || total <= 0) return 0;
  return Math.max(0, Math.min(1, absolute / total));
}

/** Completion at or past which an audiobook counts as finished. */
export const LISTENING_FINISHED_THRESHOLD = 0.999;

/** Project a book-absolute listening position onto the shared {@link Progress}. Pure. */
export function listeningProgress(absolute: number, total: number, ended = false): Progress {
  const fraction = listeningFraction(absolute, total);
  return {
    fraction,
    positionSeconds: Math.max(0, absolute),
    finished: ended || fraction >= LISTENING_FINISHED_THRESHOLD,
  };
}
