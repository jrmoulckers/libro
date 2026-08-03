import { describe, expect, it } from 'vitest';
import { countByKind, sortLibrary, type LibraryItem } from './library';

const items: LibraryItem[] = [
  { id: '2', title: 'Piranesi', author: 'Clarke, Susanna', kind: 'book' },
  { id: '1', title: 'Babel', author: 'Kuang, R. F.', kind: 'audiobook' },
  { id: '3', title: 'Ancillary Justice', author: 'Clarke, Susanna', kind: 'audiobook' },
];

describe('sortLibrary', () => {
  it('orders by author then title without mutating the input', () => {
    const sorted = sortLibrary(items);

    expect(sorted.map((item) => item.id)).toEqual(['3', '2', '1']);
    expect(items.map((item) => item.id)).toEqual(['2', '1', '3']);
  });
});

describe('countByKind', () => {
  it('counts items of a single kind', () => {
    expect(countByKind(items, 'audiobook')).toBe(2);
    expect(countByKind(items, 'book')).toBe(1);
  });
});
