import { describe, it, expect, vi } from 'vitest';
import type { Book, Progress } from '../models';
import type { ProgressSource, ProgressStore, LocalProgress } from './source';
import type { RemoteProgress } from './reconcile';
import { syncProgress, shouldPush, SyncMemory, type SyncDeps } from './sync';

function book(id: string, mediaType: Book['mediaType'] = 'audiobook'): Book {
  return {
    id,
    title: id,
    authors: [],
    mediaType,
    sourceProviderId: 'abs',
    identifiers: { 'abs:item_id': id },
  };
}

/** A fake local lane store backed by a Map. */
class FakeStore implements ProgressStore {
  map = new Map<string, Progress>();
  constructor(seed: Record<string, Progress> = {}) {
    for (const [k, v] of Object.entries(seed)) this.map.set(k, v);
  }
  async get(bookId: string): Promise<LocalProgress | null> {
    const progress = this.map.get(bookId);
    return progress ? { progress, updatedAt: null } : null;
  }
  async put(bookId: string, progress: Progress): Promise<void> {
    this.map.set(bookId, progress);
  }
}

function source(
  remote: Record<string, RemoteProgress | null>,
  push = vi.fn(async () => {}),
): ProgressSource {
  return {
    id: 'abs',
    displayName: 'Audiobookshelf',
    pullProgress: vi.fn(async (b: Book) => remote[b.id] ?? null),
    pushProgress: push,
  };
}

function deps(over: Partial<SyncDeps> & Pick<SyncDeps, 'source' | 'stores'>): SyncDeps {
  return { ...over };
}

describe('shouldPush', () => {
  it('pushes when nothing synced, flag flips, or fraction moves past epsilon', () => {
    expect(shouldPush({ fraction: 0.5, finished: false }, undefined)).toBe(true);
    expect(shouldPush({ fraction: 0.5, finished: true }, { fraction: 0.5, finished: false })).toBe(
      true,
    );
    expect(shouldPush({ fraction: 0.7, finished: false }, { fraction: 0.5, finished: false })).toBe(
      true,
    );
  });

  it('suppresses a push when the local value matches the last synced mark', () => {
    expect(
      shouldPush({ fraction: 0.505, finished: false }, { fraction: 0.5, finished: false }),
    ).toBe(false);
  });
});

describe('syncProgress — apply outcomes', () => {
  it('pulls down when the remote is further along (no local timestamp)', async () => {
    const listening = new FakeStore({ b: { fraction: 0.2, finished: false } });
    const reading = new FakeStore();
    const src = source({ b: { fraction: 0.6, finished: false, updatedAt: null } });

    const report = await syncProgress(
      [book('b')],
      deps({ source: src, stores: { reading, listening } }),
    );

    expect(report.pulledDown).toBe(1);
    expect(listening.map.get('b')?.fraction).toBeCloseTo(0.6);
  });

  it('pushes up when the local value is further and worth pushing', async () => {
    const push = vi.fn(async () => {});
    const listening = new FakeStore({
      b: { fraction: 0.8, positionSeconds: 100, finished: false },
    });
    const src = source({ b: { fraction: 0.2, finished: false, updatedAt: null } }, push);

    const report = await syncProgress(
      [book('b')],
      deps({ source: src, stores: { reading: new FakeStore(), listening } }),
    );

    expect(report.pushed).toBe(1);
    expect(push).toHaveBeenCalledOnce();
  });

  it('adopts the remote when there is no local value', async () => {
    const listening = new FakeStore();
    const src = source({ b: { fraction: 0.3, finished: false, updatedAt: null } });

    const report = await syncProgress(
      [book('b')],
      deps({ source: src, stores: { reading: new FakeStore(), listening } }),
    );

    expect(report.pulledDown).toBe(1);
    expect(listening.map.get('b')?.fraction).toBeCloseTo(0.3);
  });

  it('counts noRemote when the source has no record', async () => {
    const report = await syncProgress(
      [book('b')],
      deps({
        source: source({ b: null }),
        stores: { reading: new FakeStore(), listening: new FakeStore() },
      }),
    );
    expect(report.noRemote).toBe(1);
  });

  it('routes ebooks to the reading lane and audiobooks to listening', async () => {
    const reading = new FakeStore();
    const listening = new FakeStore();
    const src = source({
      e: { fraction: 0.5, finished: false, updatedAt: null },
      a: { fraction: 0.5, finished: false, updatedAt: null },
    });
    await syncProgress(
      [book('e', 'ebook'), book('a', 'audiobook')],
      deps({ source: src, stores: { reading, listening } }),
    );
    expect(reading.map.has('e')).toBe(true);
    expect(listening.map.has('a')).toBe(true);
  });

  it('skips books with no reconcilable lane', async () => {
    const src = source({ p: { fraction: 0.5, finished: false, updatedAt: null } });
    const pull = src.pullProgress as ReturnType<typeof vi.fn>;
    await syncProgress(
      [book('p', 'podcast')],
      deps({ source: src, stores: { reading: new FakeStore(), listening: new FakeStore() } }),
    );
    expect(pull).not.toHaveBeenCalled();
  });
});

describe('syncProgress — policy: manual conflicts', () => {
  it('records a genuine conflict without writing the store', async () => {
    const listening = new FakeStore({ b: { fraction: 0.2, finished: false } });
    const src = source({ b: { fraction: 0.7, finished: false, updatedAt: null } });

    const report = await syncProgress(
      [book('b')],
      deps({ source: src, stores: { reading: new FakeStore(), listening }, policy: 'manual' }),
    );

    expect(report.conflicts).toHaveLength(1);
    expect(report.conflicts[0]).toMatchObject({
      bookId: 'b',
      lane: 'listening',
      remoteSource: 'Audiobookshelf',
    });
    // Store untouched — still the original local value.
    expect(listening.map.get('b')?.fraction).toBeCloseTo(0.2);
  });

  it('auto mode resolves the same case instead of surfacing it', async () => {
    const listening = new FakeStore({ b: { fraction: 0.2, finished: false } });
    const src = source({ b: { fraction: 0.7, finished: false, updatedAt: null } });

    const report = await syncProgress(
      [book('b')],
      deps({ source: src, stores: { reading: new FakeStore(), listening }, policy: 'auto' }),
    );

    expect(report.conflicts).toHaveLength(0);
    expect(report.pulledDown).toBe(1);
  });
});

describe('syncProgress — isolation & anti-oscillation', () => {
  it('isolates a failing pull without aborting the sweep', async () => {
    const listening = new FakeStore({ ok: { fraction: 0.1, finished: false } });
    const src: ProgressSource = {
      id: 'abs',
      displayName: 'ABS',
      pullProgress: vi.fn(async (b: Book) => {
        if (b.id === 'bad') throw new Error('boom');
        return { fraction: 0.6, finished: false, updatedAt: null };
      }),
      pushProgress: vi.fn(async () => {}),
    };

    const report = await syncProgress(
      [book('bad'), book('ok')],
      deps({ source: src, stores: { reading: new FakeStore(), listening } }),
    );

    expect(report.errors).toHaveLength(1);
    expect(report.errors[0]!.bookId).toBe('bad');
    expect(report.pulledDown).toBe(1); // 'ok' still processed
  });

  it('does not re-push a just-pulled value on the next pass (shared memory)', async () => {
    const memory = new SyncMemory();
    const listening = new FakeStore({ b: { fraction: 0.2, finished: false } });
    const push = vi.fn(async () => {});
    // Pass 1: remote 0.6 wins → pull down; memory seeded at 0.6.
    const src1 = source({ b: { fraction: 0.6, finished: false, updatedAt: null } }, push);
    await syncProgress(
      [book('b')],
      deps({ source: src1, stores: { reading: new FakeStore(), listening }, memory }),
    );

    // Pass 2: remote now reports behind (0.2). Local is 0.6 (== last synced) →
    // keep-local, but shouldPush is false, so nothing is pushed back up.
    const src2 = source({ b: { fraction: 0.2, finished: false, updatedAt: null } }, push);
    const report2 = await syncProgress(
      [book('b')],
      deps({ source: src2, stores: { reading: new FakeStore(), listening }, memory }),
    );

    expect(push).not.toHaveBeenCalled();
    expect(report2.keptLocal).toBe(1);
  });
});
