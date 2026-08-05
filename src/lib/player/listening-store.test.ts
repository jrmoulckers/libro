import { describe, expect, it } from 'vitest';
import { InMemoryListeningStore, type ListeningPosition } from './listening-store';

const pos = (positionSeconds: number, fraction: number, finished = false): ListeningPosition => ({
  positionSeconds,
  fraction,
  finished,
});

describe('InMemoryListeningStore', () => {
  it('round-trips a position by book id', async () => {
    const store = new InMemoryListeningStore();
    await store.set('abs-1', pos(120, 0.4));
    expect(await store.get('abs-1')).toEqual(pos(120, 0.4));
    expect(await store.get('missing')).toBeUndefined();
  });

  it('clones on the way in and out (no shared references)', async () => {
    const store = new InMemoryListeningStore();
    const p = pos(60, 0.2);
    await store.set('b', p);
    p.positionSeconds = 999;
    const fetched = (await store.get('b'))!;
    fetched.fraction = 0.99;
    expect(await store.get('b')).toEqual(pos(60, 0.2));
  });

  it('lists all positions and removes/clears', async () => {
    const store = new InMemoryListeningStore();
    await store.set('a', pos(10, 0.1));
    await store.set('b', pos(20, 0.2, true));

    expect([...(await store.all()).keys()].sort()).toEqual(['a', 'b']);

    await store.remove('a');
    expect(await store.get('a')).toBeUndefined();

    await store.clear();
    expect((await store.all()).size).toBe(0);
  });
});
