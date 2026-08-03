<script lang="ts">
  import { onMount } from 'svelte';
  import {
    countByMediaType,
    groupByMediaType,
    MEDIA_TYPES,
    sortBooks,
    type Book,
    type MediaType,
  } from './lib/models';
  import { loadLibrary } from './lib/library';

  type LoadState = 'loading' | 'loaded' | 'error';

  const MEDIA_LABELS: Record<MediaType, string> = {
    ebook: 'E-books',
    audiobook: 'Audiobooks',
    podcast: 'Podcasts',
  };

  let loadState = $state<LoadState>('loading');
  let books = $state<Book[]>([]);

  // Group the sorted catalog, dropping media types with no items.
  const sections = $derived(
    [...groupByMediaType(sortBooks(books)).entries()].filter(([, items]) => items.length > 0),
  );

  function authorLine(book: Book): string {
    return book.authors.length > 0 ? book.authors.join(', ') : 'Unknown author';
  }

  onMount(async () => {
    try {
      const result = await loadLibrary();
      books = result.books;
      loadState = 'loaded';
    } catch {
      loadState = 'error';
    }
  });
</script>

<main>
  <header>
    <h1>Libro</h1>
    <p class="tagline">
      A cross-platform, pure-client media hub for books, audiobooks, and your personal library.
    </p>
  </header>

  <section class="library" aria-busy={loadState === 'loading'} aria-live="polite">
    {#if loadState === 'loading'}
      <p class="status" role="status">Loading your library…</p>
    {:else if loadState === 'error'}
      <p class="status" role="alert">Your library could not be loaded. Try reloading.</p>
    {:else if books.length === 0}
      <p class="status">Your library is empty. Importing is not wired up yet.</p>
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
            {#each items as book (`${book.sourceProviderId}:${book.id}`)}
              <li class="item">
                <span class="title">{book.title}</span>
                <span class="author">{authorLine(book)}</span>
                {#if book.series}
                  <span class="series">{book.series}</span>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    {/if}
  </section>
</main>

<style>
  main {
    margin: 0 auto;
    max-width: 60rem;
    padding: 2rem 1rem;
  }

  .tagline {
    max-width: 42rem;
  }

  /* Reserve vertical space so swapping loading/empty/loaded states doesn't shift
     surrounding content (avoids CLS). */
  .library {
    min-height: 60vh;
  }

  .status {
    margin: 0;
  }

  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    padding: 0;
    list-style: none;
  }

  .shelf {
    margin-block-start: 2rem;
  }

  .items {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: 1rem;
    padding: 0;
    list-style: none;
  }

  .item {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .title {
    font-weight: 600;
  }
</style>
