import { describe, expect, it } from 'vitest';
import { chunk, openLibraryBatchUrl, parseOpenLibrary, parseOpenLibraryBatch } from './openlibrary';

// Shaped after a live `/api/books?...jscmd=data` response, trimmed to the mapped
// fields. Two known ISBNs + one that is absent from the response (not found).
const BATCH = {
  'ISBN:9780134685991': {
    title: 'Effective Java',
    authors: [{ name: 'Joshua Bloch' }, { name: 'Neal Gafter' }],
    subjects: [{ name: 'Java (Computer program language)' }, { name: 'Programming' }],
    cover: {
      small: 'https://covers.openlibrary.org/b/id/12420356-S.jpg',
      medium: 'https://covers.openlibrary.org/b/id/12420356-M.jpg',
      large: 'https://covers.openlibrary.org/b/id/12420356-L.jpg',
    },
  },
  'ISBN:9780596007126': {
    title: 'Head First Design Patterns',
    authors: [{ name: 'Eric Freeman' }],
    // No description (jscmd=data omits it) and only a medium cover.
    cover: { medium: 'https://covers.openlibrary.org/b/id/388761-M.jpg' },
  },
};

describe('openLibraryBatchUrl', () => {
  it('builds an encoded multi-bibkey URL', () => {
    const url = openLibraryBatchUrl(['9780134685991', '9780596007126']);
    expect(url).toContain('/api/books?bibkeys=');
    expect(url).toContain('format=json');
    expect(url).toContain('jscmd=data');
    // The comma between bibkeys is percent-encoded.
    expect(url).toContain(encodeURIComponent('ISBN:9780134685991,ISBN:9780596007126'));
  });
});

describe('parseOpenLibrary', () => {
  it('maps authors, prefers the large cover, and reads subjects', () => {
    const patch = parseOpenLibrary(BATCH, '9780134685991');
    expect(patch.authors).toEqual(['Joshua Bloch', 'Neal Gafter']);
    expect(patch.coverUrl).toBe('https://covers.openlibrary.org/b/id/12420356-L.jpg');
    expect(patch.subjects).toEqual(['Java (Computer program language)', 'Programming']);
    expect(patch.description).toBeUndefined(); // jscmd=data omits it
  });

  it('falls back to a medium cover and tolerates missing fields', () => {
    const patch = parseOpenLibrary(BATCH, '9780596007126');
    expect(patch.coverUrl).toBe('https://covers.openlibrary.org/b/id/388761-M.jpg');
    expect(patch.authors).toEqual(['Eric Freeman']);
    expect(patch.subjects).toBeUndefined();
  });

  it('returns an empty patch for an ISBN absent from the response', () => {
    expect(parseOpenLibrary(BATCH, '9999999999999')).toEqual({});
    expect(parseOpenLibrary({}, '9780134685991')).toEqual({});
    expect(parseOpenLibrary(null, '9780134685991')).toEqual({});
  });

  it('maps a description when present as a string or {value}', () => {
    const json = {
      'ISBN:1': { title: 'A', description: '  Plain synopsis.  ' },
      'ISBN:2': { title: 'B', description: { value: 'Typed synopsis.' } },
    };
    expect(parseOpenLibrary(json, '1').description).toBe('Plain synopsis.');
    expect(parseOpenLibrary(json, '2').description).toBe('Typed synopsis.');
  });
});

describe('parseOpenLibraryBatch', () => {
  it('returns a patch (possibly empty) for every requested ISBN', () => {
    const map = parseOpenLibraryBatch(BATCH, ['9780134685991', '9780596007126', '0000000000000']);
    expect(map.size).toBe(3);
    expect(map.get('9780134685991')?.authors).toEqual(['Joshua Bloch', 'Neal Gafter']);
    expect(map.get('0000000000000')).toEqual({});
  });
});

describe('chunk', () => {
  it('splits into runs of at most size', () => {
    expect(chunk([1, 2, 3, 4, 5], 2)).toEqual([[1, 2], [3, 4], [5]]);
    expect(chunk([], 3)).toEqual([]);
    expect(chunk([1, 2], 0)).toEqual([[1], [2]]); // size floored to 1
  });
});
