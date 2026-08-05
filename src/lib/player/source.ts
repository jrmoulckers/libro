/**
 * Playback-source abstraction: resolve a {@link Book} to a {@link PlaybackManifest}
 * (ordered track URLs + book-absolute chapters) the player can render.
 *
 * This is the audio analog of the catalog {@link ../providers/types.Provider}: the
 * catalog layer tells us a book *exists*; a `PlaybackSource` tells us how to
 * *play* it. Different sources build the manifest differently — an Audiobookshelf
 * source hits `POST /api/items/{id}/play` ({@link ./abs-source}); the bundled demo
 * synthesizes tone clips ({@link ./sample-source}) — but both feed the one
 * `Player.svelte` rendering path. The manifest-building logic is pure/tested; only
 * the fetch (or object-URL minting) is the thin shell.
 */

import type { Book } from '../models';
import type { PlaybackManifest } from './timeline';

/** Resolves a book into a directly-playable multi-track manifest. */
export interface PlaybackSource {
  /** Stable id, matching a {@link ../providers/types.Provider.id} it plays for. */
  id: string;
  /** Build the playable manifest for a book. May fetch (async). */
  resolve(book: Book): Promise<PlaybackManifest>;
}
