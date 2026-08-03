<script lang="ts">
  import { countByKind, sortLibrary, type LibraryItem } from './lib/library';

  const items: LibraryItem[] = [];
  const shelf = sortLibrary(items);
  const audiobooks = countByKind(items, 'audiobook');
</script>

<main>
  <h1>Libro</h1>
  <p>A cross-platform, pure-client media hub for books, audiobooks, and your personal library.</p>

  {#if shelf.length === 0}
    <p>Your library is empty. Importing is not wired up yet.</p>
  {:else}
    <ul>
      {#each shelf as item (item.id)}
        <li>{item.title} — {item.author}</li>
      {/each}
    </ul>
    <p>{audiobooks} audiobook(s).</p>
  {/if}
</main>

<style>
  main {
    margin: 0 auto;
    max-width: 60rem;
    padding: 2rem 1rem;
  }
</style>
