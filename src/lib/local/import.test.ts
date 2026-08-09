import { describe, expect, it } from 'vitest';
import type { ParsedEpub } from '../epub/epub';
import {
  buildLocalBook,
  createLocalProvider,
  importEpubFiles,
  localBookId,
  LOCAL_PROVIDER_ID,
  type EpubParser,
  type ImportableFile,
} from './import';
import { InMemoryLocalStore } from './store';

function epubFile(name: string, bytes = 'epub'): ImportableFile {
  return new File([bytes], name, { type: 'application/epub+zip' });
}

const PARSED_FULL: ParsedEpub = {
  metadata: {
    title: 'Effective Java',
    authors: ['Joshua Bloch', 'Second Author'],
    description: 'A description.',
    identifiers: { isbn13: '9780134685991' },
    series: 'The Java Series',
  },
  coverBytes: new Uint8Array([1, 2, 3]),
};

/** A parser that returns fixed metadata regardless of input (offline, zip-free). */
const fakeParse: EpubParser = async () => PARSED_FULL;

describe('localBookId', () => {
  it('is stable for the same name + size and differs otherwise', () => {
    const a = localBookId(epubFile('book.epub'));
    expect(localBookId(epubFile('book.epub'))).toBe(a);
    expect(localBookId(epubFile('other.epub'))).not.toBe(a);
    expect(a.startsWith('local-')).toBe(true);
  });
});

describe('buildLocalBook', () => {
  it('maps metadata, sets localcover url, and stamps local:source', () => {
    const file = epubFile('effective-java.epub');
    const book = buildLocalBook('local-1', file, PARSED_FULL);

    expect(book).toMatchObject({
      id: 'local-1',
      title: 'Effective Java',
      authors: ['Joshua Bloch', 'Second Author'],
      mediaType: 'ebook',
      sourceProviderId: LOCAL_PROVIDER_ID,
      series: 'The Java Series',
      coverUrl: 'localcover:local-1',
    });
    expect(book.identifiers).toMatchObject({
      isbn13: '9780134685991',
      'local:source': 'effective-java.epub',
    });
  });

  it('falls back to the filename when the OPF had no title', () => {
    const parsed: ParsedEpub = { metadata: { title: '', authors: [], identifiers: {} } };
    const book = buildLocalBook('local-2', epubFile('My Great Book.epub'), parsed);
    expect(book.title).toBe('My Great Book');
    expect(book.coverUrl).toBeUndefined();
  });
});

describe('importEpubFiles', () => {
  it('imports EPUBs, persists them, and stores the raw file + cover', async () => {
    const store = new InMemoryLocalStore();
    const file = epubFile('effective-java.epub');

    const { imported, errors } = await importEpubFiles([file], store, fakeParse);

    expect(errors).toEqual([]);
    expect(imported).toHaveLength(1);
    const id = imported[0]!.id;
    expect(await store.getFile(id)).toBe(file);
    expect(await store.getCover(id)).toBeInstanceOf(Blob);
    expect((await store.listBooks()).map((b) => b.title)).toEqual(['Effective Java']);
  });

  it('skips non-EPUB files with a reason', async () => {
    const store = new InMemoryLocalStore();
    const { imported, errors } = await importEpubFiles([epubFile('notes.txt')], store, fakeParse);
    expect(imported).toEqual([]);
    expect(errors).toEqual([{ name: 'notes.txt', reason: 'Not an .epub file' }]);
  });

  it('skips an unparseable EPUB with the parser error message', async () => {
    const store = new InMemoryLocalStore();
    const failing: EpubParser = async () => {
      throw new Error('not a valid EPUB (could not unzip)');
    };
    const { imported, errors } = await importEpubFiles([epubFile('broken.epub')], store, failing);
    expect(imported).toEqual([]);
    expect(errors).toEqual([{ name: 'broken.epub', reason: 'not a valid EPUB (could not unzip)' }]);
  });

  it('dedupes duplicates within the batch and against the store', async () => {
    const store = new InMemoryLocalStore();
    const first = await importEpubFiles([epubFile('dup.epub')], store, fakeParse);
    expect(first.imported).toHaveLength(1);

    // Same name+size again (in a batch with itself) -> both dedupe away.
    const second = await importEpubFiles(
      [epubFile('dup.epub'), epubFile('dup.epub')],
      store,
      fakeParse,
    );
    expect(second.imported).toEqual([]);
    expect(second.errors).toEqual([
      { name: 'dup.epub', reason: 'Already in your library' },
      { name: 'dup.epub', reason: 'Already in your library' },
    ]);
    expect(await store.listBooks()).toHaveLength(1);
  });

  it('one bad file never aborts the batch', async () => {
    const store = new InMemoryLocalStore();
    const { imported, errors } = await importEpubFiles(
      [epubFile('bad.txt'), epubFile('good.epub')],
      store,
      fakeParse,
    );
    expect(imported.map((b) => b.identifiers?.['local:source'])).toEqual(['good.epub']);
    expect(errors).toEqual([{ name: 'bad.txt', reason: 'Not an .epub file' }]);
  });
});

describe('createLocalProvider', () => {
  it('lists persisted books with the catalog capability and no network', async () => {
    const store = new InMemoryLocalStore();
    await importEpubFiles([epubFile('effective-java.epub')], store, fakeParse);

    const provider = createLocalProvider(store);
    expect(provider.id).toBe(LOCAL_PROVIDER_ID);
    expect([...provider.capabilities]).toEqual(['catalog']);
    expect((await provider.listBooks()).map((b) => b.title)).toEqual(['Effective Java']);
  });
});
