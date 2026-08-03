/**
 * Standalone sanity check for the pure Media Session timeline helpers.
 * Runnable with `node --experimental-strip-types` (Node 24+), no test runner /
 * browser needed — it only exercises the DOM-free mapping functions. The DOM
 * wiring in AudioPlayer.tsx is covered by `npm run build` (tsc typecheck).
 */
import assert from "node:assert/strict";
import {
  mediaMetadataInput,
  positionStateInput,
  nextChapterStart,
  prevChapterStart,
} from "../src/audioTimeline.ts";
import type { AudioChapter, Book } from "../src/types.ts";

const book: Book = {
  id: "b1",
  title: "The Fellowship of the Ring",
  authors: ["J.R.R. Tolkien"],
  series: "The Lord of the Rings",
  cover_url: "https://example.com/cover.jpg",
  identifiers: {},
  media_type: "Audiobook",
  source_provider_id: "abs",
};

// metadata: pulls from the book.
const md = mediaMetadataInput(book, "fallback");
assert.equal(md.title, "The Fellowship of the Ring");
assert.equal(md.artist, "J.R.R. Tolkien");
assert.equal(md.album, "The Lord of the Rings");
assert.deepEqual(md.artwork, [{ src: "https://example.com/cover.jpg" }]);

// metadata: localcover:// artwork is dropped (OS can't fetch it).
const localCover = mediaMetadataInput(
  { ...book, cover_url: "localcover://b1" },
  undefined,
);
assert.deepEqual(localCover.artwork, []);

// metadata: fallbacks when no book.
const fb = mediaMetadataInput(undefined, "My Book");
assert.equal(fb.title, "My Book");
assert.equal(fb.artist, "Unknown author");
assert.equal(fb.album, "My Book");

// positionState: valid on a known total, null on unknown duration.
const ps = positionStateInput(3600, 120, 1.5);
assert.deepEqual(ps, { duration: 3600, position: 120, playbackRate: 1.5 });
assert.equal(positionStateInput(0, 0, 1), null);
assert.equal(positionStateInput(NaN, 10, 1), null);
// clamps position past the end; normalizes bad rate.
assert.deepEqual(positionStateInput(100, 999, 0), {
  duration: 100,
  position: 100,
  playbackRate: 1,
});

// chapter navigation on the unified timeline.
const chapters: AudioChapter[] = [
  { title: "One", start: 0, end: 100 },
  { title: "Two", start: 100, end: 250 },
  { title: "Three", start: 250, end: 400 },
];
// next: from ch1 -> start of ch2; from last -> null (fall back to skip).
assert.equal(nextChapterStart(chapters, 50), 100);
assert.equal(nextChapterStart(chapters, 300), null);
assert.equal(nextChapterStart([], 10), null);
// prev: >3s into a chapter restarts it; <=3s steps back; first stays at 0.
assert.equal(prevChapterStart(chapters, 160), 100); // 60s into ch2 -> restart ch2
assert.equal(prevChapterStart(chapters, 101), 0); // 1s into ch2 -> ch1 start
assert.equal(prevChapterStart(chapters, 2), 0); // start of ch1 -> stays
assert.equal(prevChapterStart([], 10), null);

console.log("media-session helper checks passed");
