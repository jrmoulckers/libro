import { describe, expect, it } from 'vitest';
import { mapAbsPlaybackSession, resolveStreamUrl, AbsPlaybackError } from './abs-source';
import { assembleTimeline } from './timeline';

const TOKEN = 'tok en/+';

// Minified /api/items/{id}/play session: 2 tracks, chapters straddling the
// track boundary at 300s, and one malformed chapter that must be skipped.
const SESSION = {
  audioTracks: [
    { index: 0, contentUrl: '/s/item/li_1/file/aaa.mp3', duration: 300, mimeType: 'audio/mpeg' },
    { index: 1, contentUrl: 'https://cdn.example.com/bbb.mp3?x=1', duration: 200 },
  ],
  chapters: [
    { id: 0, start: 0, end: 250, title: 'Opening' },
    { id: 1, start: 250, end: 450, title: '' },
    { id: 2, start: 450, end: 500 },
    { id: 3, title: 'broken' },
  ],
};

describe('resolveStreamUrl', () => {
  it('resolves a server-relative path and appends the token', () => {
    expect(resolveStreamUrl('https://abs.test/', '/s/f.mp3', 'abc')).toBe(
      'https://abs.test/s/f.mp3?token=abc',
    );
  });

  it('keeps an absolute URL and uses & when a query already exists', () => {
    expect(resolveStreamUrl('https://abs.test', 'https://cdn/f.mp3?a=1', 'abc')).toBe(
      'https://cdn/f.mp3?a=1&token=abc',
    );
  });

  it('url-encodes the token and omits it when empty', () => {
    expect(resolveStreamUrl('https://abs.test', '/f.mp3', TOKEN)).toBe(
      `https://abs.test/f.mp3?token=${encodeURIComponent(TOKEN)}`,
    );
    expect(resolveStreamUrl('https://abs.test', '/f.mp3', '')).toBe('https://abs.test/f.mp3');
  });
});

describe('mapAbsPlaybackSession', () => {
  const manifest = mapAbsPlaybackSession(SESSION, 'https://abs.test', 'abc');

  it('resolves track URLs with the token and lays out the timeline', () => {
    expect(manifest.tracks).toHaveLength(2);
    expect(manifest.tracks[0].url).toBe('https://abs.test/s/item/li_1/file/aaa.mp3?token=abc');
    expect(manifest.tracks[1].url).toBe('https://cdn.example.com/bbb.mp3?x=1&token=abc');
    expect(manifest.tracks.map((t) => t.durationSeconds)).toEqual([300, 200]);
    expect(manifest.tracks[0].mimeType).toBe('audio/mpeg');
    // Offsets are derived by assembleTimeline, not carried on the raw manifest.
    const timeline = assembleTimeline(manifest.tracks, manifest.chapters);
    expect(timeline.tracks.map((t) => t.startOffset)).toEqual([0, 300]);
    expect(timeline.totalDuration).toBe(500);
  });

  it('maps chapters, defaults blank titles, and skips malformed ones', () => {
    expect(manifest.chapters).toHaveLength(3);
    expect(manifest.chapters[0]).toEqual({ title: 'Opening', startAbsolute: 0, endAbsolute: 250 });
    expect(manifest.chapters[1].title).toBe('Chapter 2'); // blank title defaulted
    expect(manifest.chapters[2].title).toBe('Chapter 3');
  });

  it('throws AbsPlaybackError when there are no audio tracks', () => {
    expect(() => mapAbsPlaybackSession({ audioTracks: [] }, 'https://abs.test', 'abc')).toThrow(
      AbsPlaybackError,
    );
  });
});
