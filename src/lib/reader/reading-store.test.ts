import { describe, expect, it } from 'vitest';
import { makePosition } from './locator';
import { InMemoryReadingStore } from './reading-store';

describe('InMemoryReadingStore', () => {
  it('round-trips a position by book id', async () => {
    const store = new InMemoryReadingStore();
    const pos = makePosition(2, 0.5, 5);

    await store.set('abs-1', pos);
    expect(await store.get('abs-1')).toEqual(pos);
    expect(await store.get('missing')).toBeUndefined();
  });

  it('clones on the way in and out (no shared references)', async () => {
    const store = new InMemoryReadingStore();
    const pos = makePosition(1, 0.2, 4);
    await store.set('b', pos);

    const fetched = (await store.get('b'))!;
    fetched.scrollFraction = 0.99;
    expect((await store.get('b'))!.scrollFraction).toBe(0.2);
  });

  it('lists all positions and removes/clears', async () => {
    const store = new InMemoryReadingStore();
    await store.set('a', makePosition(0, 0.1, 3));
    await store.set('b', makePosition(1, 0.2, 3));

    expect([...(await store.all()).keys()].sort()).toEqual(['a', 'b']);

    await store.remove('a');
    expect(await store.get('a')).toBeUndefined();

    await store.clear();
    expect((await store.all()).size).toBe(0);
  });
});
