import { describe, expect, it } from 'vitest';
import type { Book } from '../models';
import {
  assembleTimeline,
  endOfChapterAbsolute,
  fadeMultiplier,
  listeningFraction,
  listeningProgress,
  locateAbsolute,
  mediaMetadataInput,
  nextChapterStart,
  positionStateInput,
  prevChapterStart,
  sleepRemainingSeconds,
  trackToAbsolute,
  type Chapter,
  type PlaybackTrack,
} from './timeline';

const TRACKS: PlaybackTrack[] = [
  { url: 't1', durationSeconds: 100 },
  { url: 't2', durationSeconds: 200 },
  { url: 't3', durationSeconds: 50 },
];

// Chapters straddle track boundaries (boundary at 100 and 300).
const CHAPTERS: Chapter[] = [
  { title: 'One', startAbsolute: 0, endAbsolute: 120 },
  { title: 'Two', startAbsolute: 120, endAbsolute: 320 },
  { title: 'Three', startAbsolute: 320, endAbsolute: 350 },
];

describe('assembleTimeline', () => {
  it('computes cumulative start offsets and total duration', () => {
    const tl = assembleTimeline(TRACKS, CHAPTERS);
    expect(tl.tracks.map((t) => t.startOffset)).toEqual([0, 100, 300]);
    expect(tl.totalDuration).toBe(350);
    expect(tl.chapters).toHaveLength(3);
  });

  it('clamps negative / NaN / infinite durations to 0', () => {
    const tl = assembleTimeline([
      { url: 'a', durationSeconds: -5 },
      { url: 'b', durationSeconds: Number.NaN },
      { url: 'c', durationSeconds: 30 },
    ]);
    expect(tl.tracks.map((t) => t.startOffset)).toEqual([0, 0, 0]);
    expect(tl.totalDuration).toBe(30);
  });
});

describe('locateAbsolute / trackToAbsolute', () => {
  const tl = assembleTimeline(TRACKS);

  it('locates a position inside a track', () => {
    expect(locateAbsolute(tl.tracks, 150)).toEqual({ trackIndex: 1, offsetInTrack: 50 });
  });

  it('resolves a boundary value to the START of the later track', () => {
    expect(locateAbsolute(tl.tracks, 100)).toEqual({ trackIndex: 1, offsetInTrack: 0 });
    expect(locateAbsolute(tl.tracks, 300)).toEqual({ trackIndex: 2, offsetInTrack: 0 });
  });

  it('clamps below zero and past the end (to the last track)', () => {
    expect(locateAbsolute(tl.tracks, -10)).toEqual({ trackIndex: 0, offsetInTrack: 0 });
    expect(locateAbsolute(tl.tracks, 999)).toEqual({ trackIndex: 2, offsetInTrack: 50 });
  });

  it('is the inverse of trackToAbsolute', () => {
    expect(trackToAbsolute(tl.tracks, 2, 25)).toBe(325);
    const { trackIndex, offsetInTrack } = locateAbsolute(tl.tracks, 325);
    expect(trackToAbsolute(tl.tracks, trackIndex, offsetInTrack)).toBe(325);
  });

  it('handles an empty timeline', () => {
    expect(locateAbsolute([], 10)).toEqual({ trackIndex: 0, offsetInTrack: 0 });
  });
});

describe('sleep timer math', () => {
  it('rounds remaining seconds up and never goes negative', () => {
    expect(sleepRemainingSeconds(10_400, 10_000)).toBe(1);
    expect(sleepRemainingSeconds(9_000, 10_000)).toBe(0);
  });

  it('finds the end of the chapter containing a position (across a boundary)', () => {
    expect(endOfChapterAbsolute(CHAPTERS, 200)).toBe(320); // chapter Two spans 120..320
    expect(endOfChapterAbsolute(CHAPTERS, 50)).toBe(120);
    expect(endOfChapterAbsolute(CHAPTERS, 999)).toBeNull();
    expect(endOfChapterAbsolute([], 10)).toBeNull();
  });

  it('fades linearly over the final seconds', () => {
    expect(fadeMultiplier(10, 5)).toBe(1);
    expect(fadeMultiplier(2.5, 5)).toBe(0.5);
    expect(fadeMultiplier(0, 5)).toBe(0);
    expect(fadeMultiplier(3, 0)).toBe(1);
  });
});

describe('chapter navigation', () => {
  it('finds the next chapter start or null at the end', () => {
    expect(nextChapterStart(CHAPTERS, 50)).toBe(120);
    expect(nextChapterStart(CHAPTERS, 330)).toBeNull();
    expect(nextChapterStart([], 10)).toBeNull();
  });

  it('restarts the current chapter when >3s in, else steps back', () => {
    expect(prevChapterStart(CHAPTERS, 200)).toBe(120); // 80s into ch Two -> restart
    expect(prevChapterStart(CHAPTERS, 121)).toBe(0); // 1s into ch Two -> prior start
    expect(prevChapterStart(CHAPTERS, 1)).toBe(0); // first chapter -> its own start
    expect(prevChapterStart([], 10)).toBeNull();
  });
});

describe('media session inputs', () => {
  const book: Book = {
    id: 'b',
    title: 'Babel',
    authors: ['R. F. Kuang', 'Someone'],
    mediaType: 'audiobook',
    sourceProviderId: 'mock',
    series: 'Babel Series',
  };

  it('builds metadata with joined authors and series as album', () => {
    const input = mediaMetadataInput(book);
    expect(input.title).toBe('Babel');
    expect(input.artist).toBe('R. F. Kuang, Someone');
    expect(input.album).toBe('Babel Series');
  });

  it('keeps only OS-fetchable artwork (http/data), dropping local schemes', () => {
    expect(mediaMetadataInput({ ...book, coverUrl: 'https://x/c.jpg' }).artwork).toEqual([
      { src: 'https://x/c.jpg' },
    ]);
    expect(mediaMetadataInput({ ...book, coverUrl: 'localcover:abc' }).artwork).toEqual([]);
    expect(mediaMetadataInput({ ...book, coverUrl: 'blob:xyz' }).artwork).toEqual([]);
  });

  it('guards invalid position state and clamps a valid one', () => {
    expect(positionStateInput(0, 10, 1)).toBeNull();
    expect(positionStateInput(Number.NaN, 10, 1)).toBeNull();
    expect(positionStateInput(300, 999, 0)).toEqual({
      duration: 300,
      position: 300,
      playbackRate: 1,
    });
  });
});

describe('progress projection', () => {
  it('computes clamped fraction', () => {
    expect(listeningFraction(150, 300)).toBe(0.5);
    expect(listeningFraction(999, 300)).toBe(1);
    expect(listeningFraction(10, 0)).toBe(0);
  });

  it('projects a Progress, marking finished at the end', () => {
    expect(listeningProgress(150, 300)).toEqual({
      fraction: 0.5,
      positionSeconds: 150,
      finished: false,
    });
    expect(listeningProgress(300, 300).finished).toBe(true);
    expect(listeningProgress(0, 300, true).finished).toBe(true);
  });
});
