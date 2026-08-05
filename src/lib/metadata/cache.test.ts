import { describe, expect, it } from 'vitest';
import { InMemoryMetadataCache } from './cache';

describe('InMemoryMetadataCache', () => {
  it('round-trips patches, isolates clones, and lists/clears them', async () => {
    const cache = new InMemoryMetadataCache();
    expect(await cache.get('isbn-a')).toBeUndefined();

    const patch = { description: 'd', subjects: ['x'] };
    await cache.set('isbn-a', patch);
    await cache.set('isbn-b', {}); // negative result is cached too

    const got = await cache.get('isbn-a');
    expect(got).toEqual(patch);
    // Stored value is cloned — mutating the input must not corrupt the cache.
    patch.subjects.push('y');
    expect((await cache.get('isbn-a'))?.subjects).toEqual(['x']);

    const all = await cache.all();
    expect(all.size).toBe(2);
    expect(all.get('isbn-b')).toEqual({});

    await cache.clear();
    expect((await cache.all()).size).toBe(0);
  });
});
