import { describe, it, expect } from 'vitest';
import type { Book, Progress } from '../models';
import { InMemoryReadingStore } from '../reader/reading-store';
import { InMemoryListeningStore } from '../player/listening-store';
import {
  laneForBook,
  readingPositionToProgress,
  progressToReadingPosition,
  listeningPositionToProgress,
  progressToListeningPosition,
  readingProgressStore,
  listeningProgressStore,
} from './lanes';

function book(id: string, mediaType: Book['mediaType']): Book {
  return { id, title: id, authors: [], mediaType, sourceProviderId: 'p', identifiers: {} };
}

describe('laneForBook', () => {
  it('routes by media type, skipping the rest', () => {
    expect(laneForBook(book('a', 'audiobook'))).toBe('listening');
    expect(laneForBook(book('b', 'ebook'))).toBe('reading');
    expect(laneForBook(book('c', 'podcast'))).toBeNull();
  });
});

describe('reading lane conversions', () => {
  it('round-trips a spine position through Progress', () => {
    const pos = { spineIndex: 3, scrollFraction: 0.5, fraction: 0.44, finished: false };
    const progress = readingPositionToProgress(pos);
    expect(progress.fraction).toBeCloseTo(0.44);
    const back = progressToReadingPosition(progress);
    expect(back.spineIndex).toBe(3);
    expect(back.scrollFraction).toBeCloseTo(0.5);
  });

  it('falls back to a coarse position when the progress has no locator', () => {
    const back = progressToReadingPosition({ fraction: 0.6, finished: false });
    expect(back.spineIndex).toBe(0);
    expect(back.scrollFraction).toBeCloseTo(0.6);
    expect(back.fraction).toBeCloseTo(0.6);
  });
});

describe('listening lane conversions', () => {
  it('round-trips seconds through Progress', () => {
    const pos = { positionSeconds: 1234, fraction: 0.3, finished: false };
    const progress = listeningPositionToProgress(pos);
    expect(progress.positionSeconds).toBe(1234);
    const back = progressToListeningPosition(progress);
    expect(back.positionSeconds).toBe(1234);
    expect(back.fraction).toBeCloseTo(0.3);
  });

  it('defaults seconds to 0 when Progress carries none', () => {
    expect(progressToListeningPosition({ fraction: 0.2, finished: false }).positionSeconds).toBe(0);
  });
});

describe('store adapters', () => {
  it('reading adapter reads/writes through the underlying store', async () => {
    const store = new InMemoryReadingStore();
    const adapter = readingProgressStore(store);
    expect(await adapter.get('x')).toBeNull();

    const progress: Progress = {
      fraction: 0.5,
      locator: '{"spineIndex":2,"scrollFraction":0.25}',
      finished: false,
    };
    await adapter.put('x', progress);
    const round = await adapter.get('x');
    expect(round?.progress.fraction).toBeCloseTo(0.5);
    expect(round?.updatedAt).toBeNull();
    // The native store received a decoded ReadingPosition.
    expect((await store.get('x'))?.spineIndex).toBe(2);
  });

  it('listening adapter reads/writes through the underlying store', async () => {
    const store = new InMemoryListeningStore();
    const adapter = listeningProgressStore(store);
    await adapter.put('y', { fraction: 0.8, positionSeconds: 999, finished: false });
    expect((await store.get('y'))?.positionSeconds).toBe(999);
    expect((await adapter.get('y'))?.progress.fraction).toBeCloseTo(0.8);
  });
});
