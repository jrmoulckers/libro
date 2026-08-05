import { describe, it, expect } from 'vitest';
import { mapAbsMediaProgress, ABS_ITEM_ID_KEY } from './abs-source';

const ME = {
  id: 'user-1',
  mediaProgress: [
    {
      id: 'mp-1',
      libraryItemId: 'li-abc',
      duration: 3600,
      progress: 0.42,
      currentTime: 1512,
      isFinished: false,
      lastUpdate: 1_700_000_000_000, // epoch ms
    },
    {
      id: 'mp-2',
      libraryItemId: 'li-fin',
      progress: 1,
      currentTime: 5400,
      isFinished: true,
      lastUpdate: 1_700_000_500_000,
    },
    {
      id: 'mp-3',
      libraryItemId: 'li-pod',
      episodeId: 'ep-9', // podcast episode row — must be ignored for a book
      progress: 0.9,
      isFinished: false,
    },
  ],
};

describe('mapAbsMediaProgress', () => {
  it('maps a matching in-progress record, converting lastUpdate ms → seconds', () => {
    const r = mapAbsMediaProgress(ME, 'li-abc');
    expect(r).not.toBeNull();
    expect(r?.fraction).toBeCloseTo(0.42);
    expect(r?.positionSeconds).toBe(1512);
    expect(r?.finished).toBe(false);
    expect(r?.updatedAt).toBe(1_700_000_000);
  });

  it('reports finished when isFinished is set', () => {
    expect(mapAbsMediaProgress(ME, 'li-fin')?.finished).toBe(true);
  });

  it('returns null when there is no record for the item', () => {
    expect(mapAbsMediaProgress(ME, 'li-missing')).toBeNull();
  });

  it('ignores podcast-episode rows for a book id', () => {
    expect(mapAbsMediaProgress(ME, 'li-pod')).toBeNull();
  });

  it('tolerates a malformed payload', () => {
    expect(mapAbsMediaProgress(null, 'x')).toBeNull();
    expect(mapAbsMediaProgress({}, 'x')).toBeNull();
    expect(mapAbsMediaProgress({ mediaProgress: 'nope' }, 'x')).toBeNull();
  });

  it('defaults updatedAt to null when lastUpdate is absent', () => {
    const me = { mediaProgress: [{ libraryItemId: 'x', progress: 0.5 }] };
    const r = mapAbsMediaProgress(me, 'x');
    expect(r?.updatedAt).toBeNull();
    expect(r?.positionSeconds).toBeUndefined();
  });

  it('exports the identifier key it matches on', () => {
    expect(ABS_ITEM_ID_KEY).toBe('abs:item_id');
  });
});
