<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    bookKey,
    countByMediaType,
    groupByMediaType,
    MEDIA_TYPES,
    sortBooks,
    type Book,
    type MediaType,
  } from './lib/models';
  import {
    appRegistry,
    defaultListeningStore,
    defaultLocalStore,
    defaultPlaybackSource,
    defaultReadingStore,
    enrichLibrary,
    loadLibrary,
    resolveLibraryConflict,
    syncLibrary,
  } from './lib/library';
  import type { AbsConfig } from './lib/providers/audiobookshelf';
  import type { ConflictResolution } from './lib/sync/reconcile';
  import type { ProgressConflict, ConflictChoice } from './lib/sync/conflict';
  import { importEpubFiles, LOCAL_PROVIDER_ID, type ImportableFile } from './lib/local/import';
  import { isLocalCoverUrl, localCoverObjectUrl } from './lib/local/store';
  import { positionToProgress } from './lib/reader/locator';
  import {
    applyTheme,
    nextTheme,
    persistTheme,
    prefersDark,
    readStoredTheme,
    resolveInitialTheme,
    themeLabel,
    type Theme,
  } from './lib/pwa/theme';
  import {
    isStandalone,
    shouldShowInstall,
    type BeforeInstallPromptEvent,
  } from './lib/pwa/install';

  type LoadState = 'loading' | 'loaded' | 'error';
  type ReaderComponent = typeof import('./lib/reader/Reader.svelte').default;
  type PlayerComponent = typeof import('./lib/player/Player.svelte').default;
  type ImportRow = { name: string; ok: boolean; detail: string };

  const MEDIA_LABELS: Record<MediaType, string> = {
    ebook: 'E-books',
    audiobook: 'Audiobooks',
    podcast: 'Podcasts',
  };

  // One on-device store, shared by the local provider (reads) and imports (writes).
  const store = defaultLocalStore();
  // Per-book reading positions, written by the reader and read back for progress.
  const readingStore = defaultReadingStore();
  // Per-book listening positions + the source that resolves playable audio.
  const listeningStore = defaultListeningStore();
  const playbackSource = defaultPlaybackSource();

  let loadState = $state<LoadState>('loading');
  let books = $state<Book[]>([]);
  let importing = $state(false);
  let importRows = $state<ImportRow[]>([]);
  let coverSrc = $state<Record<string, string>>({});
  // Metadata enrichment is opt-in and default-on: the sources (Open Library /
  // Google Books) are CORS-open, keyless, cached by ISBN, and only ever *fill*
  // gaps — so it's safe to run by default. The user can still turn it off here.
  let enrichEnabled = $state(true);
  let enriching = $state(false);

  // Progress sync (P8): reconcile device-local reading/listening positions with a
  // remote tracker (Audiobookshelf). `auto` auto-resolves every case; `manual`
  // surfaces genuine, unorderable conflicts in a banner for the user to settle.
  // No ABS server is configured in this mock-only build, so the sweep is a no-op
  // here — `absConfigs` is the composition point where on-device settings would
  // supply real connectors (base URL + API token), never baked-in secrets.
  const absConfigs: AbsConfig[] = [];
  let conflictResolution = $state<ConflictResolution>('auto');
  let syncing = $state(false);
  let conflicts = $state<ProgressConflict[]>([]);

  // The reader is code-split: its module (and fflate) load only on first open.
  let ReaderComp = $state<ReaderComponent | null>(null);
  let activeBook = $state<Book | null>(null);
  // The player is likewise code-split: loaded only when an audiobook is opened.
  let PlayerComp = $state<PlayerComponent | null>(null);
  let listeningBook = $state<Book | null>(null);

  let fileInput: HTMLInputElement;
  let objectUrls: string[] = [];

  // --- PWA: theme toggle (dataset.theme only — values arrive with @jrm/tokens) ---
  let theme = $state<Theme>(resolveInitialTheme(prefersDark(), readStoredTheme()));
  function toggleTheme(): void {
    theme = nextTheme(theme);
    applyTheme(theme);
    persistTheme(theme);
  }

  // --- PWA: install affordance (feature-detected; iOS Safari has no prompt) ---
  let deferredPrompt = $state<BeforeInstallPromptEvent | null>(null);
  let installed = $state(false);
  const standalone = isStandalone();
  const installVisible = $derived(
    shouldShowInstall({ promptAvailable: deferredPrompt !== null, installed, standalone }),
  );

  function onBeforeInstallPrompt(event: Event): void {
    // Suppress the mini-infobar; we show our own accessible affordance instead.
    event.preventDefault();
    deferredPrompt = event as BeforeInstallPromptEvent;
  }
  function onAppInstalled(): void {
    installed = true;
    deferredPrompt = null;
  }
  async function install(): Promise<void> {
    const prompt = deferredPrompt;
    if (!prompt) return;
    deferredPrompt = null; // a prompt can only be used once
    try {
      await prompt.prompt();
      await prompt.userChoice;
    } catch {
      // The user dismissed the prompt or it was already consumed — nothing to do.
    }
  }

  // Group the sorted catalog, dropping media types with no items.
  const sections = $derived(
    [...groupByMediaType(sortBooks(books)).entries()].filter(([, items]) => items.length > 0),
  );

  function authorLine(book: Book): string {
    return book.authors.length > 0 ? book.authors.join(', ') : 'Unknown author';
  }

  /** Whether this book's bytes are retrievable so it can be opened in the reader. */
  function canRead(book: Book): boolean {
    return book.mediaType === 'ebook' && book.sourceProviderId === LOCAL_PROVIDER_ID;
  }

  /** Whether this book can be played in the audio player (any audiobook). */
  function canListen(book: Book): boolean {
    return book.mediaType === 'audiobook';
  }

  /** A short, human label for a book's read/listen progress, or '' if untouched. */
  function progressLabel(book: Book): string {
    if (!book.progress) return '';
    if (book.progress.finished) return 'Finished';
    const pct = Math.round(book.progress.fraction * 100);
    return `${pct}% ${book.mediaType === 'audiobook' ? 'listened' : 'read'}`;
  }

  /** A short label for one side of a progress conflict (finished, or percent). */
  function conflictSideLabel(side: { fraction: number; finished: boolean }): string {
    return side.finished ? 'is finished' : `at ${Math.round(side.fraction * 100)}%`;
  }

  /** Fold persisted reading + listening positions into each book's `progress`. */
  async function withProgress(list: readonly Book[]): Promise<Book[]> {
    const [reading, listening] = await Promise.all([readingStore.all(), listeningStore.all()]);
    return list.map((book) => {
      const reader = reading.get(book.id);
      if (reader) return { ...book, progress: positionToProgress(reader) };
      const listener = listening.get(book.id);
      if (listener) {
        return {
          ...book,
          progress: {
            fraction: listener.fraction,
            positionSeconds: listener.positionSeconds,
            finished: listener.finished,
          },
        };
      }
      return book;
    });
  }

  /**
   * Resolve cover URLs for the current catalog. `localcover:` covers become
   * temporary object URLs from the store; connector `http(s)` covers are used
   * verbatim. Previously-created object URLs are revoked to avoid leaks.
   */
  async function refreshCovers(list: readonly Book[]): Promise<void> {
    const previous = objectUrls;
    const next: Record<string, string> = {};
    const created: string[] = [];

    for (const book of list) {
      if (!book.coverUrl) continue;
      if (isLocalCoverUrl(book.coverUrl)) {
        const url = await localCoverObjectUrl(store, book.coverUrl);
        if (url) {
          next[bookKey(book)] = url;
          created.push(url);
        }
      } else {
        next[bookKey(book)] = book.coverUrl;
      }
    }

    coverSrc = next;
    objectUrls = created;
    for (const url of previous) URL.revokeObjectURL(url);
  }

  async function reload(): Promise<void> {
    const result = await loadLibrary(appRegistry(store));
    books = await withProgress(result.books);
    await refreshCovers(books);
  }

  /**
   * Fill metadata gaps from the public APIs *after* first paint, then update the
   * cards in place. Best-effort: never blocks rendering and never throws (a failed
   * lookup is isolated inside `enrichLibrary`). No-op when disabled or empty.
   */
  async function enrichInBackground(): Promise<void> {
    if (!enrichEnabled || enriching || books.length === 0) return;
    enriching = true;
    try {
      const { books: enriched } = await enrichLibrary(books);
      books = await withProgress(enriched);
      await refreshCovers(books);
    } catch {
      // Enrichment is a nice-to-have; leave the un-enriched library as-is.
    } finally {
      enriching = false;
    }
  }

  /** When the user turns enrichment on, run a pass immediately. */
  function onEnrichToggle(): void {
    if (enrichEnabled) void enrichInBackground();
  }

  /**
   * Reconcile local progress with the configured remote tracker(s) *after* first
   * paint, then refresh the cards. Best-effort: never blocks rendering and never
   * throws (per-book failures are isolated inside `syncLibrary`). A no-op when no
   * ABS connector is configured (the mock-only demo). Under `manual` policy,
   * genuine conflicts land in the banner instead of auto-applying.
   */
  async function syncInBackground(): Promise<void> {
    if (syncing || books.length === 0 || absConfigs.length === 0) return;
    syncing = true;
    try {
      const report = await syncLibrary(books, { abs: absConfigs, policy: conflictResolution });
      conflicts = report.conflicts;
      // Reconciled winners were written to the lane stores; reflect them on cards.
      books = await withProgress(books);
      await refreshCovers(books);
    } catch {
      // Sync is best-effort; leave the local view untouched on failure.
    } finally {
      syncing = false;
    }
  }

  /** Re-run the sweep when the conflict policy changes (e.g. auto → manual). */
  function onPolicyChange(): void {
    conflicts = [];
    void syncInBackground();
  }

  /** Apply the user's choice for one pending conflict and drop it from the banner. */
  async function onConflictChoice(
    conflict: ProgressConflict,
    choice: ConflictChoice,
  ): Promise<void> {
    try {
      await resolveLibraryConflict(conflict, choice, { readingStore, listeningStore });
      conflicts = conflicts.filter((c) => c.bookId !== conflict.bookId);
      books = await withProgress(books);
      await refreshCovers(books);
    } catch {
      // Leave the conflict in place so the user can retry.
    }
  }

  /**
   * Open a book in the reader. The reader component (and its `fflate` dependency)
   * is dynamically imported the first time, keeping it out of the main entry chunk.
   */
  async function openReader(book: Book): Promise<void> {
    if (!ReaderComp) {
      const module = await import('./lib/reader/Reader.svelte');
      ReaderComp = module.default;
    }
    activeBook = book;
  }

  /** Close the reader and refresh the library so updated progress shows. */
  async function closeReader(): Promise<void> {
    activeBook = null;
    await reload();
  }

  /**
   * Open an audiobook in the player. The player component is dynamically imported
   * the first time, keeping it out of the main entry chunk (like the reader).
   */
  async function openPlayer(book: Book): Promise<void> {
    if (!PlayerComp) {
      const module = await import('./lib/player/Player.svelte');
      PlayerComp = module.default;
    }
    listeningBook = book;
  }

  /** Close the player and refresh the library so updated progress shows. */
  async function closePlayer(): Promise<void> {
    listeningBook = null;
    await reload();
  }

  /** Import a picked set of files, then refresh the library and status list. */
  async function runImport(files: ImportableFile[]): Promise<void> {
    if (files.length === 0) return;
    importing = true;
    try {
      const { imported, errors } = await importEpubFiles(files, store);
      await reload();
      importRows = [
        ...imported.map((book) => ({
          name: book.identifiers?.['local:source'] ?? book.title,
          ok: true,
          detail: 'Imported',
        })),
        ...errors.map((skip) => ({ name: skip.name, ok: false, detail: skip.reason })),
      ];
    } finally {
      importing = false;
    }
  }

  /**
   * Open the file picker. Uses the File System Access API (`showOpenFilePicker`)
   * when available as a progressive enhancement, and always falls back to the
   * native `<input type="file">` — the baseline that works everywhere.
   */
  async function pickFiles(): Promise<void> {
    const picker = (
      window as unknown as {
        showOpenFilePicker?: (options?: unknown) => Promise<Array<{ getFile(): Promise<File> }>>;
      }
    ).showOpenFilePicker;

    if (picker) {
      try {
        const handles = await picker({
          multiple: true,
          types: [{ description: 'EPUB books', accept: { 'application/epub+zip': ['.epub'] } }],
        });
        await runImport(await Promise.all(handles.map((handle) => handle.getFile())));
        return;
      } catch (error) {
        // User dismissed the picker — do nothing; otherwise fall back to the input.
        if (error instanceof DOMException && error.name === 'AbortError') return;
      }
    }

    fileInput.click();
  }

  async function onInputChange(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = input.files ? Array.from(input.files) : [];
    await runImport(files);
    input.value = ''; // allow re-picking the same file
  }

  onMount(async () => {
    window.addEventListener('beforeinstallprompt', onBeforeInstallPrompt);
    window.addEventListener('appinstalled', onAppInstalled);
    try {
      await reload();
      loadState = 'loaded';
      // Enrich AFTER first paint so it never delays the initial render.
      void enrichInBackground();
      // Reconcile progress with any configured remote tracker, also post-paint.
      void syncInBackground();
    } catch {
      loadState = 'error';
    }
  });

  onDestroy(() => {
    window.removeEventListener('beforeinstallprompt', onBeforeInstallPrompt);
    window.removeEventListener('appinstalled', onAppInstalled);
    for (const url of objectUrls) URL.revokeObjectURL(url);
  });
</script>

<main>
  <header>
    <div class="masthead">
      <h1>Libro</h1>
      <div class="shell-controls">
        <button
          type="button"
          class="control"
          onclick={toggleTheme}
          aria-label={`Theme: ${themeLabel(theme)}. Activate to switch to ${themeLabel(nextTheme(theme))}.`}
        >
          <span class="control-label">Theme</span>
          <span class="control-value">{themeLabel(theme)}</span>
        </button>
        {#if installVisible}
          <button type="button" class="control install" onclick={install}> Install app </button>
        {/if}
      </div>
    </div>
    <p class="tagline">
      A cross-platform, pure-client media hub for books, audiobooks, and your personal library.
    </p>
  </header>

  <section class="import" aria-labelledby="import-heading">
    <h2 id="import-heading">Your books</h2>
    <p class="import-hint">Import your own DRM-free EPUB files. They stay on this device.</p>
    <button type="button" onclick={pickFiles} disabled={importing}>
      {importing ? 'Importing…' : 'Import EPUB(s)'}
    </button>
    <input
      bind:this={fileInput}
      class="visually-hidden"
      type="file"
      accept=".epub,application/epub+zip"
      multiple
      tabindex="-1"
      aria-hidden="true"
      onchange={onInputChange}
    />

    <p class="enrich-control">
      <label>
        <input type="checkbox" bind:checked={enrichEnabled} onchange={onEnrichToggle} />
        Enrich metadata from public catalogs
      </label>
      {#if enriching}
        <span class="enrich-status" role="status">Enriching…</span>
      {/if}
    </p>

    <p class="sync-control">
      <label for="conflict-policy">Progress conflicts</label>
      <select
        id="conflict-policy"
        bind:value={conflictResolution}
        onchange={onPolicyChange}
        disabled={syncing}
      >
        <option value="auto">Resolve automatically</option>
        <option value="manual">Let me choose</option>
      </select>
      {#if syncing}
        <span class="enrich-status" role="status">Syncing…</span>
      {/if}
    </p>

    {#if conflicts.length > 0}
      <section class="conflicts" aria-labelledby="conflicts-heading">
        <h3 id="conflicts-heading">Sync conflicts ({conflicts.length})</h3>
        <ul>
          {#each conflicts as conflict (conflict.bookId)}
            <li>
              <p class="conflict-title">{conflict.title}</p>
              <p class="conflict-detail">
                This device {conflictSideLabel(conflict.local)} · {conflict.remoteSource}
                {conflictSideLabel(conflict.remote)}
              </p>
              <div class="conflict-actions">
                <button type="button" onclick={() => onConflictChoice(conflict, 'keep_local')}>
                  Keep this device
                </button>
                <button type="button" onclick={() => onConflictChoice(conflict, 'use_remote')}>
                  Use {conflict.remoteSource}
                </button>
                <button type="button" onclick={() => onConflictChoice(conflict, 'keep_furthest')}>
                  Keep furthest
                </button>
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if importRows.length > 0}
      <ul class="import-status" aria-label="Import results" aria-live="polite">
        {#each importRows as row (row.name + row.detail)}
          <li class:ok={row.ok} class:skip={!row.ok}>
            <span class="import-marker" aria-hidden="true">{row.ok ? '✓' : '✕'}</span>
            <span class="import-name">{row.name}</span>
            <span class="import-detail">{row.detail}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="library" aria-busy={loadState === 'loading'} aria-live="polite">
    {#if loadState === 'loading'}
      <p class="status" role="status">Loading your library…</p>
    {:else if loadState === 'error'}
      <p class="status" role="alert">Your library could not be loaded. Try reloading.</p>
    {:else if books.length === 0}
      <p class="status">Your library is empty. Import an EPUB to get started.</p>
    {:else}
      <ul class="counts" aria-label="Library summary">
        {#each MEDIA_TYPES as type (type)}
          <li>{MEDIA_LABELS[type]}: {countByMediaType(books, type)}</li>
        {/each}
      </ul>

      {#each sections as [type, items] (type)}
        <section class="shelf" aria-labelledby={`shelf-${type}`}>
          <h2 id={`shelf-${type}`}>{MEDIA_LABELS[type]}</h2>
          <ul class="items">
            {#each items as book (bookKey(book))}
              <li class="item">
                <div class="cover" aria-hidden="true">
                  {#if coverSrc[bookKey(book)]}
                    <img src={coverSrc[bookKey(book)]} alt="" loading="lazy" />
                  {/if}
                </div>
                <span class="title">{book.title}</span>
                <span class="author">{authorLine(book)}</span>
                {#if book.series}
                  <span class="series">{book.series}</span>
                {/if}
                {#if progressLabel(book)}
                  <span class="progress" class:finished={book.progress?.finished}>
                    {progressLabel(book)}
                  </span>
                {/if}
                {#if canRead(book)}
                  <button type="button" class="read" onclick={() => openReader(book)}>
                    Read
                  </button>
                {/if}
                {#if canListen(book)}
                  <button type="button" class="read" onclick={() => openPlayer(book)}>
                    Listen
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    {/if}
  </section>
</main>

{#if ReaderComp && activeBook}
  <ReaderComp book={activeBook} {store} {readingStore} onClose={closeReader} />
{/if}

{#if PlayerComp && listeningBook}
  <PlayerComp book={listeningBook} source={playbackSource} {listeningStore} onClose={closePlayer} />
{/if}

<style>
  main {
    margin: 0 auto;
    max-width: 60rem;
    padding: var(--spacing-3xl) var(--spacing-lg);
  }

  header {
    padding-block-end: var(--spacing-lg);
    border-block-end: 1px solid var(--semantic-border-default);
  }

  /* Title + controls on one baseline row; the install button appearing changes
     row width, not height, so it does not shift the content below (no CLS). */
  .masthead {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--spacing-sm);
  }

  .shell-controls {
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
  }

  /* The theme cycle control shows its current value; it is the studio "default"
     (neutral) button — the Royal Violet primary is reserved for real actions. */
  .control {
    display: inline-flex;
    align-items: baseline;
    gap: var(--spacing-xs);
  }

  .control-label {
    font-size: var(--text-overline-size);
    font-weight: var(--text-overline-weight);
    letter-spacing: var(--text-overline-letter-spacing);
    text-transform: uppercase;
    color: var(--semantic-text-secondary);
  }

  .control-value {
    font-weight: var(--font-weight-semibold);
    color: var(--semantic-text-primary);
  }

  /* Primary actions wear the one brand action color (Royal Violet). */
  .install {
    color: var(--button-primary-text);
    background: var(--button-primary-bg);
    border-color: var(--button-primary-border);
  }

  .install:hover:not(:disabled) {
    background: var(--button-primary-hover-bg);
  }

  .tagline {
    max-width: 42rem;
    color: var(--semantic-text-secondary);
  }

  /* Import panel is an elevated card. */
  .import {
    margin-block-start: var(--spacing-2xl);
    padding: var(--card-padding);
    background: var(--semantic-background-elevated);
    border: 1px solid var(--semantic-border-default);
    border-radius: var(--card-radius);
    box-shadow: var(--shadow-lift);
  }

  .import-hint {
    max-width: 42rem;
    color: var(--semantic-text-secondary);
  }

  .import-status {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-xs);
    margin-block-start: var(--spacing-lg);
    padding: 0;
    list-style: none;
  }

  .import-status li {
    display: flex;
    align-items: baseline;
    gap: var(--spacing-sm);
  }

  .import-marker {
    font-weight: var(--font-weight-bold);
  }

  .import-status li.ok .import-marker {
    color: var(--semantic-status-positive);
  }

  .import-status li.skip .import-marker {
    color: var(--semantic-status-negative);
  }

  .import-name {
    font-weight: var(--font-weight-semibold);
  }

  .import-detail {
    margin-inline-start: auto;
    color: var(--semantic-text-secondary);
  }

  .enrich-control {
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
    margin-block-start: var(--spacing-sm);
  }

  .enrich-control label {
    display: flex;
    align-items: center;
    gap: var(--spacing-xs);
  }

  .enrich-status {
    color: var(--semantic-text-secondary);
  }

  .sync-control {
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
    margin-block-start: var(--spacing-sm);
  }

  /* A conflict banner is a negative-status surface. Text stays primary for
     contrast; the coral border + heading carry the signal (not hue alone). */
  .conflicts {
    margin-block-start: var(--spacing-lg);
    border: 1px solid var(--semantic-status-negative);
    border-radius: var(--radius-md);
    padding: var(--spacing-lg);
    background: var(--semantic-background-raised);
  }

  .conflicts h3 {
    margin: 0 0 var(--spacing-sm);
  }

  .conflicts ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--spacing-lg);
  }

  .conflict-title {
    margin: 0;
    font-weight: var(--font-weight-semibold);
  }

  .conflict-detail {
    margin: var(--spacing-xs) 0 var(--spacing-sm);
    color: var(--semantic-text-secondary);
  }

  .conflict-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--spacing-sm);
  }

  /* Reserve vertical space so swapping loading/empty/loaded states doesn't shift
     surrounding content (avoids CLS). */
  .library {
    min-height: 60vh;
  }

  .status {
    margin: 0;
    color: var(--semantic-text-secondary);
  }

  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--spacing-sm);
    margin-block-start: var(--spacing-lg);
    padding: 0;
    list-style: none;
  }

  .counts li {
    padding: var(--pill-padding-y) var(--pill-padding-x);
    font-size: var(--text-label-size);
    color: var(--pill-text);
    background: var(--pill-bg);
    border: 1px solid var(--pill-border);
    border-radius: var(--pill-radius);
  }

  .shelf {
    margin-block-start: var(--spacing-3xl);
  }

  .items {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: var(--spacing-lg);
    margin-block-start: var(--spacing-lg);
    padding: 0;
    list-style: none;
  }

  /* Each book is an elevated card that lifts on hover. */
  .item {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-xs);
    padding: var(--card-padding);
    background: var(--semantic-background-elevated);
    border: 1px solid var(--semantic-border-default);
    border-radius: var(--card-radius);
    box-shadow: var(--shadow-lift);
    transition: border-color var(--duration-state) var(--easing-standard);
  }

  .item:hover {
    border-color: var(--semantic-interactive-default);
  }

  /* Fixed aspect ratio reserves layout space before the cover loads (no CLS). */
  .cover {
    aspect-ratio: 2 / 3;
    overflow: hidden;
    background: var(--semantic-background-secondary);
    border-radius: var(--radius-sm);
  }

  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .title {
    font-weight: var(--font-weight-semibold);
    color: var(--semantic-text-primary);
  }

  .author {
    color: var(--semantic-text-secondary);
  }

  /* Series is a light accent line (Crown Gold text, darkened for contrast). */
  .series {
    font-size: var(--text-label-size);
    color: var(--semantic-accent-ink);
  }

  /* Progress is a neutral pill; "Finished" adds a positive border + check so the
     completed state is not signalled by color alone. */
  .progress {
    align-self: flex-start;
    margin-block-start: var(--spacing-xs);
    padding: var(--pill-padding-y) var(--pill-padding-x);
    font-size: var(--text-label-size);
    color: var(--semantic-text-primary);
    background: var(--semantic-background-raised);
    border: 1px solid var(--semantic-border-default);
    border-radius: var(--pill-radius);
  }

  .progress.finished {
    border-color: var(--semantic-status-positive);
  }

  .progress.finished::before {
    content: '✓ ';
  }

  /* Read / Listen are primary actions — the one Royal Violet fill. */
  .read {
    margin-block-start: var(--spacing-sm);
    color: var(--button-primary-text);
    background: var(--button-primary-bg);
    border-color: var(--button-primary-border);
  }

  .read:hover:not(:disabled) {
    background: var(--button-primary-hover-bg);
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
</style>
