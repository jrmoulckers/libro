import { describe, expect, it } from 'vitest';
import {
  bookGaps,
  combinePatches,
  extractIsbn,
  hasGaps,
  mergePatch,
  normalizeIsbn,
  patchFields,
  type MetadataPatch,
} from './types';
import type { Book } from '../models';

function book(overrides: Partial<Book> = {}): Book {
  return {
    id: '1',
    title: 'Effective Java',
    authors: [],
    mediaType: 'ebook',
    sourceProviderId: 'abs',
    ...overrides,
  };
}

describe('normalizeIsbn', () => {
  it('strips hyphens/spaces and accepts valid ISBN-10/13', () => {
    expect(normalizeIsbn('978-0-13-468599-1')).toBe('9780134685991');
    expect(normalizeIsbn(' 0 134 68599 7 ')).toBe('0134685997');
    expect(normalizeIsbn('080442957x')).toBe('080442957X'); // trailing X upper-cased
  });

  it('rejects non-ISBN strings', () => {
    expect(normalizeIsbn('12345')).toBeNull();
    expect(normalizeIsbn('9770134685991')).toBeNull(); // 13 digits, bad prefix
    expect(normalizeIsbn('not-an-isbn')).toBeNull();
  });
});

describe('extractIsbn', () => {
  it('prefers ISBN-13 and reads varied identifier schemes', () => {
    expect(extractIsbn({ identifiers: { isbn_10: '0134685997', isbn_13: '9780134685991' } })).toBe(
      '9780134685991',
    );
    expect(extractIsbn({ identifiers: { 'dcterms:identifier': 'urn:isbn:9780134685991' } })).toBe(
      '9780134685991',
    );
    expect(extractIsbn({ identifiers: { 'opds:acquisition_url': 'https://x/y' } })).toBeNull();
  });

  it('falls back to ISBN-10 and returns null with no ISBN', () => {
    expect(extractIsbn({ identifiers: { ISBN: '0-13-468599-7' } })).toBe('0134685997');
    expect(extractIsbn({ identifiers: { asin: 'B000123' } })).toBeNull();
    expect(extractIsbn({})).toBeNull();
  });
});

describe('gap detection', () => {
  it('reports each missing enrichable field', () => {
    expect(hasGaps(book())).toBe(true);
    expect([...bookGaps(book())].sort()).toEqual(
      ['authors', 'coverUrl', 'description', 'series', 'subjects'].sort(),
    );
  });

  it('is false when every enrichable field is present', () => {
    const full = book({
      authors: ['A'],
      coverUrl: 'c',
      description: 'd',
      series: 's',
      subjects: ['x'],
    });
    expect(hasGaps(full)).toBe(false);
    expect(bookGaps(full).size).toBe(0);
  });
});

describe('combinePatches', () => {
  it('keeps primary fields and borrows only what it lacks', () => {
    const ol: MetadataPatch = { coverUrl: 'ol-cover', authors: ['A'] };
    const gb: MetadataPatch = { coverUrl: 'gb-cover', description: 'gb-desc', subjects: ['sci'] };
    expect(combinePatches(ol, gb)).toEqual({
      coverUrl: 'ol-cover', // primary wins
      authors: ['A'],
      description: 'gb-desc', // filled from fallback
      subjects: ['sci'],
    });
  });

  it('reports the fields a patch can supply', () => {
    expect([...patchFields({ description: 'd', authors: [] })]).toEqual(['description']);
    expect(patchFields({}).size).toBe(0);
  });
});

describe('mergePatch', () => {
  it('fills only gaps and never clobbers existing data', () => {
    const original = book({ coverUrl: 'existing-cover', authors: ['Real Author'] });
    const patch: MetadataPatch = {
      coverUrl: 'new-cover',
      authors: ['Wrong'],
      description: 'filled',
      series: 'The Series',
      subjects: ['java'],
    };
    const merged = mergePatch(original, patch);

    expect(merged.coverUrl).toBe('existing-cover'); // preserved
    expect(merged.authors).toEqual(['Real Author']); // preserved
    expect(merged.description).toBe('filled'); // gap filled
    expect(merged.series).toBe('The Series');
    expect(merged.subjects).toEqual(['java']);
  });

  it('returns the same reference when nothing changes', () => {
    const complete = book({
      authors: ['A'],
      coverUrl: 'c',
      description: 'd',
      series: 's',
      subjects: ['x'],
    });
    expect(mergePatch(complete, { description: 'other' })).toBe(complete);

    const noUsefulPatch = book({ authors: ['A'] });
    // Patch supplies nothing for the remaining gaps -> unchanged reference.
    expect(mergePatch(noUsefulPatch, { authors: ['ignored'] })).toBe(noUsefulPatch);
  });
});
