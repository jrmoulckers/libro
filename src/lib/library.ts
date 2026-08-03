/**
 * Minimal domain seed for the library index. Real cataloguing, persistence, and
 * import pipelines are not built yet — this exists so the toolchain has something
 * typed and tested to compile.
 */

export type MediaKind = 'book' | 'audiobook';

export interface LibraryItem {
  id: string;
  title: string;
  author: string;
  kind: MediaKind;
}

/** Sort by author, then title, using locale-aware comparison. */
export function sortLibrary(items: readonly LibraryItem[]): LibraryItem[] {
  return [...items].sort(
    (a, b) => a.author.localeCompare(b.author) || a.title.localeCompare(b.title),
  );
}

export function countByKind(items: readonly LibraryItem[], kind: MediaKind): number {
  return items.filter((item) => item.kind === kind).length;
}
