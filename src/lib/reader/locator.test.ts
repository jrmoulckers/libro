import { describe, expect, it } from 'vitest';
import {
  decodeLocator,
  encodeLocator,
  FINISHED_THRESHOLD,
  makePosition,
  positionToProgress,
  readingFraction,
} from './locator';

describe('readingFraction', () => {
  it('treats each spine document as an equal slice', () => {
    expect(readingFraction(0, 0, 4)).toBe(0);
    expect(readingFraction(1, 0, 4)).toBe(0.25);
    expect(readingFraction(2, 0.5, 4)).toBe(0.625);
  });

  it('clamps out-of-range inputs and guards an empty spine', () => {
    expect(readingFraction(0, -1, 4)).toBe(0);
    expect(readingFraction(10, 2, 4)).toBe(1);
    expect(readingFraction(0, 0.5, 0)).toBe(0);
    expect(readingFraction(0, NaN, 4)).toBe(0);
  });
});

describe('makePosition', () => {
  it('computes fraction and marks finished only at the end of the last doc', () => {
    const mid = makePosition(1, 0.5, 4);
    expect(mid.fraction).toBe(0.375);
    expect(mid.finished).toBe(false);

    const end = makePosition(3, 1, 4);
    expect(end.finished).toBe(true);
    expect(end.fraction).toBe(1);
  });

  it('is not finished at the end of a non-final document', () => {
    expect(makePosition(2, 1, 4).finished).toBe(false);
    expect(FINISHED_THRESHOLD).toBeLessThan(1);
  });
});

describe('locator encode/decode', () => {
  it('round-trips a position', () => {
    const encoded = encodeLocator({ spineIndex: 2, scrollFraction: 0.4 });
    expect(decodeLocator(encoded)).toEqual({ spineIndex: 2, scrollFraction: 0.4 });
  });

  it('returns undefined for malformed or missing input', () => {
    expect(decodeLocator(undefined)).toBeUndefined();
    expect(decodeLocator('not json')).toBeUndefined();
    expect(decodeLocator('{"spineIndex":1}')).toBeUndefined();
    expect(decodeLocator('42')).toBeUndefined();
  });
});

describe('positionToProgress', () => {
  it('projects onto the shared Progress model', () => {
    const progress = positionToProgress(makePosition(3, 1, 4));
    expect(progress.fraction).toBe(1);
    expect(progress.finished).toBe(true);
    expect(decodeLocator(progress.locator)).toEqual({ spineIndex: 3, scrollFraction: 1 });
  });
});
