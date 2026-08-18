<script lang="ts">
  /**
   * In-app EPUB reader — a lazily-mounted modal over the library.
   *
   * This component is the reader's *shell*: it retrieves the stored EPUB bytes
   * ({@link LocalStore.getFile}), opens them through the lazy {@link ./render}
   * module (which is the only place `fflate` is pulled in), and drives chapter
   * navigation + position persistence. All the heavy logic (spine/TOC assembly,
   * path resolution, HTML rewriting, fraction math) lives in pure, unit-tested
   * modules; this file is the thin, browser-only glue and is itself dynamically
   * imported from `App.svelte` so it and `fflate` stay out of the main entry chunk.
   *
   * Each chapter's XHTML is injected into a **sandboxed iframe** (`srcdoc`, no
   * `allow-scripts`) so the book's markup and CSS can't script the app or leak out
   * of its frame; `allow-same-origin` is kept only so the object URLs the parent
   * mints for the book's images/styles will load.
   */
  import { onDestroy, onMount } from 'svelte';
  import type { Book } from '../models';
  import type { LocalStore } from '../local/store';
  import { makePosition, readingFraction } from './locator';
  import type { ReadingStore } from './reading-store';
  import type { OpenedEpub, RenderedChapter } from './render';

  interface Props {
    book: Book;
    store: LocalStore;
    readingStore: ReadingStore;
    onClose: () => void;
  }

  const { book, store, readingStore, onClose }: Props = $props();

  type Status = 'loading' | 'ready' | 'error';

  let status = $state<Status>('loading');
  let opened = $state<OpenedEpub | null>(null);
  let index = $state(0);
  let rendered = $state<RenderedChapter | null>(null);
  let showToc = $state(false);
  let pct = $state(0);

  let iframeEl = $state<HTMLIFrameElement>();
  let closeButton = $state<HTMLButtonElement>();
  let pendingScroll = 0;
  let lastPersist = 0;

  const spineCount = $derived(opened?.spineCount ?? 0);
  const chapterTitle = $derived(opened ? opened.chapterLabel(index) : '');

  /** Move to a spine index, optionally restoring a scroll fraction on load. */
  function go(target: number, restoreFraction = 0): void {
    if (!opened) return;
    const next = Math.min(Math.max(target, 0), opened.spineCount - 1);
    rendered?.revoke();
    index = next;
    pendingScroll = restoreFraction;
    rendered = opened.renderChapter(next);
    pct = Math.round(readingFraction(next, restoreFraction, opened.spineCount) * 100);
    showToc = false;
  }

  /** Persist the current position (throttled by the caller for scroll). */
  async function persist(): Promise<void> {
    const doc = iframeEl?.contentDocument;
    if (!opened || !doc) return;
    const el = doc.scrollingElement ?? doc.documentElement;
    const denom = Math.max(el.scrollHeight - el.clientHeight, 0);
    const frac = denom > 0 ? el.scrollTop / denom : 0;
    const position = makePosition(index, frac, opened.spineCount);
    pct = Math.round(position.fraction * 100);
    await readingStore.set(book.id, position);
  }

  function onScroll(): void {
    const now = Date.now();
    if (now - lastPersist < 400) return;
    lastPersist = now;
    void persist();
  }

  /** Fires whenever a new chapter's srcdoc finishes parsing in the iframe. */
  function handleFrameLoad(): void {
    const doc = iframeEl?.contentDocument;
    if (!doc) return;
    const el = doc.scrollingElement ?? doc.documentElement;
    if (pendingScroll > 0) {
      el.scrollTop = pendingScroll * Math.max(el.scrollHeight - el.clientHeight, 0);
      pendingScroll = 0;
    }
    doc.addEventListener('scroll', onScroll, { passive: true });
    void persist();
  }

  function handleKey(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      onClose();
    } else if (event.key === 'ArrowRight') {
      go(index + 1);
    } else if (event.key === 'ArrowLeft') {
      go(index - 1);
    }
  }

  onMount(async () => {
    closeButton?.focus();
    try {
      const blob = await store.getFile(book.id);
      if (!blob) {
        status = 'error';
        return;
      }
      const { openEpub } = await import('./render');
      const bytes = new Uint8Array(await blob.arrayBuffer());
      opened = await openEpub(bytes);

      const saved = await readingStore.get(book.id);
      go(saved?.spineIndex ?? 0, saved?.scrollFraction ?? 0);
      status = 'ready';
    } catch {
      status = 'error';
    }
  });

  onDestroy(() => {
    rendered?.revoke();
  });
</script>

<svelte:window onkeydown={handleKey} />

<div class="backdrop">
  <div class="reader" role="dialog" aria-modal="true" aria-label={`Reading ${book.title}`}>
    <header class="bar">
      <div class="meta">
        <span class="book-title">{book.title}</span>
        {#if status === 'ready'}
          <span class="chapter" aria-live="polite">
            {chapterTitle} · {index + 1} of {spineCount} · {pct}%
          </span>
        {/if}
      </div>
      <div class="actions">
        {#if status === 'ready' && opened && opened.toc.length > 0}
          <button
            type="button"
            onclick={() => (showToc = !showToc)}
            aria-expanded={showToc}
            aria-controls="reader-toc"
          >
            Contents
          </button>
        {/if}
        <button type="button" bind:this={closeButton} onclick={onClose}>Close</button>
      </div>
    </header>

    <div class="body">
      {#if status === 'loading'}
        <p class="status" role="status">Opening book…</p>
      {:else if status === 'error'}
        <p class="status" role="alert">This book could not be opened.</p>
      {:else}
        {#if showToc && opened}
          <nav id="reader-toc" class="toc" aria-label="Table of contents">
            <ul>
              {#each opened.toc as entry (entry.index + entry.title)}
                <li>
                  <button
                    type="button"
                    class:current={entry.index === index}
                    onclick={() => go(entry.index)}
                  >
                    {entry.title}
                  </button>
                </li>
              {/each}
            </ul>
          </nav>
        {/if}

        <iframe
          bind:this={iframeEl}
          class="page"
          title={`${book.title} — ${chapterTitle}`}
          sandbox="allow-same-origin"
          srcdoc={rendered?.srcdoc ?? ''}
          onload={handleFrameLoad}
        ></iframe>
      {/if}
    </div>

    {#if status === 'ready'}
      <footer class="nav">
        <button type="button" onclick={() => go(index - 1)} disabled={index <= 0}>
          ← Previous
        </button>
        <button type="button" onclick={() => go(index + 1)} disabled={index >= spineCount - 1}>
          Next →
        </button>
      </footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: stretch;
    justify-content: center;
    /* Functional dimming scrim (not a themed surface). */
    background: rgba(0, 0, 0, 0.5);
    z-index: 100;
  }

  .reader {
    display: flex;
    flex-direction: column;
    width: min(60rem, 100%);
    height: 100%;
    color: var(--semantic-text-primary);
    background: var(--semantic-background-primary);
  }

  .bar,
  .nav {
    display: flex;
    align-items: center;
    gap: var(--spacing-lg);
    padding: var(--spacing-sm) var(--spacing-lg);
  }

  .bar {
    justify-content: space-between;
    background: var(--semantic-background-elevated);
    border-block-end: 1px solid var(--semantic-border-default);
  }

  .nav {
    justify-content: space-between;
    border-block-start: 1px solid var(--semantic-border-default);
  }

  .meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .book-title {
    font-weight: var(--font-weight-semibold);
    color: var(--semantic-text-primary);
  }

  .chapter {
    font-size: var(--text-label-size);
    color: var(--semantic-text-secondary);
  }

  .actions {
    display: flex;
    gap: var(--spacing-sm);
  }

  .body {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
  }

  .toc {
    flex: 0 0 auto;
    width: 16rem;
    max-width: 40%;
    overflow: auto;
    background: var(--semantic-background-secondary);
    border-inline-end: 1px solid var(--semantic-border-default);
  }

  .toc ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .toc button {
    display: block;
    width: 100%;
    min-height: 0;
    text-align: start;
    padding: var(--spacing-sm) var(--spacing-lg);
    color: var(--semantic-text-secondary);
    background: transparent;
    border: 0;
    border-radius: 0;
  }

  .toc button:hover {
    background: var(--semantic-background-raised);
  }

  .toc button.current {
    font-weight: var(--font-weight-bold);
    color: var(--semantic-text-primary);
  }

  .page {
    flex: 1 1 auto;
    width: 100%;
    height: 100%;
    border: 0;
    background: var(--semantic-background-elevated);
  }

  .status {
    margin: auto;
    color: var(--semantic-text-secondary);
  }
</style>
