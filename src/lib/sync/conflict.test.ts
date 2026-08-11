import { describe, it, expect } from 'vitest';
import type { Progress } from '../models';
import type { ProgressStore, LocalProgress } from './source';
import {
  ConflictStore,
  resolveConflictProgress,
  resolveProgressConflict,
  type ProgressConflict,
} from './conflict';
import { SyncMemory } from './sync';

function conflict(local: Progress, remote: Progress): ProgressConflict {
  return { bookId: 'b', title: 'B', lane: 'listening', remoteSource: 'ABS', local, remote };
}

class FakeStore implements ProgressStore {
  map = new Map<string, Progress>();
  async get(bookId: string): Promise<LocalProgress | null> {
    const progress = this.map.get(bookId);
    return progress ? { progress, updatedAt: null } : null;
  }
  async put(bookId: string, progress: Progress): Promise<void> {
    this.map.set(bookId, progress);
  }
}

describe('resolveConflictProgress', () => {
  const c = conflict({ fraction: 0.3, finished: false }, { fraction: 0.8, finished: false });

  it('keep_local / use_remote pick their side', () => {
    expect(resolveConflictProgress(c, 'keep_local').fraction).toBeCloseTo(0.3);
    expect(resolveConflictProgress(c, 'use_remote').fraction).toBeCloseTo(0.8);
  });

  it('keep_furthest picks the larger fraction (ties keep local)', () => {
    expect(resolveConflictProgress(c, 'keep_furthest').fraction).toBeCloseTo(0.8);
    const tie = conflict({ fraction: 0.5, finished: false }, { fraction: 0.5, finished: false });
    expect(resolveConflictProgress(tie, 'keep_furthest').fraction).toBeCloseTo(0.5);
  });
});

describe('ConflictStore', () => {
  it('replaces, lists, gets, and clears by book id', () => {
    const store = new ConflictStore();
    const a = {
      ...conflict({ fraction: 0.1, finished: false }, { fraction: 0.9, finished: false }),
      bookId: 'a',
    };
    const b = {
      ...conflict({ fraction: 0.2, finished: false }, { fraction: 0.8, finished: false }),
      bookId: 'b',
    };
    store.replaceAll([a, b]);
    expect(store.size()).toBe(2);
    expect(store.get('a')).toEqual(a);
    store.clear('a');
    expect(store.size()).toBe(1);
    expect(store.list()[0]!.bookId).toBe('b');
  });
});

describe('resolveProgressConflict', () => {
  it('writes the chosen winner into the lane store and seeds memory', async () => {
    const listening = new FakeStore();
    const memory = new SyncMemory();
    const c = conflict({ fraction: 0.3, finished: false }, { fraction: 0.8, finished: false });

    const winner = await resolveProgressConflict(c, 'use_remote', {
      stores: { reading: new FakeStore(), listening },
      memory,
    });

    expect(winner.fraction).toBeCloseTo(0.8);
    expect(listening.map.get('b')?.fraction).toBeCloseTo(0.8);
    expect(memory.get('b')).toEqual({ fraction: 0.8, finished: false });
  });
});
