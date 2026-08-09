/**
 * Bundled **sample** playback source — a zero-network, zero-asset audiobook so the
 * multi-track player is exercisable in the demo without a real server.
 *
 * The blueprint committed a few public-domain sample WAVs to `public/`. To protect
 * the 2048 KB budget we instead **synthesize** the audio at runtime: a tiny pure
 * PCM tone encoder ({@link encodeToneWav}) produces short mono WAV clips that
 * `resolve()` wraps in object URLs. Nothing ships in `dist/` — the clips exist only
 * in memory while the demo book is open.
 *
 * The layout ({@link sampleManifestLayout}) is deliberately multi-track with
 * chapters that **straddle track boundaries**, so the demo actually exercises
 * cross-boundary seek / chapter-jump / auto-advance. Both the encoder and the
 * layout are pure and unit-tested; only the object-URL minting is the thin shell.
 */

import type { Book } from '../models';
import { assembleTimeline, type Chapter, type PlaybackManifest } from './timeline';
import type { PlaybackSource } from './source';

/** The id this source plays for — matches the mock catalog provider. */
export const SAMPLE_SOURCE_ID = 'mock';

/** A synthetic track spec (before its audio is synthesized). */
interface SampleTrackSpec {
  durationSeconds: number;
  frequencyHz: number;
}

/** The demo layout: 3 tone tracks + chapters that cross the track boundaries. */
export function sampleManifestLayout(): { tracks: SampleTrackSpec[]; chapters: Chapter[] } {
  // Boundaries fall at 8s and 18s; chapters 2 and 3 straddle them.
  return {
    tracks: [
      { durationSeconds: 8, frequencyHz: 220 },
      { durationSeconds: 10, frequencyHz: 277 },
      { durationSeconds: 6, frequencyHz: 330 },
    ],
    chapters: [
      { title: 'Chapter 1 — Overture', startAbsolute: 0, endAbsolute: 5 },
      { title: 'Chapter 2 — Crossing', startAbsolute: 5, endAbsolute: 14 },
      { title: 'Chapter 3 — Finale', startAbsolute: 14, endAbsolute: 24 },
    ],
  };
}

/**
 * Encode a mono 16-bit PCM sine tone as a WAV byte array. Pure (no DOM/audio):
 * builds the 44-byte RIFF/WAVE header + PCM data. Kept small (low sample rate,
 * short durations) so a demo clip is only a few KB in memory.
 */
export function encodeToneWav(seconds: number, frequencyHz: number, sampleRate = 8000): Uint8Array {
  const frames = Math.max(0, Math.floor(seconds * sampleRate));
  const bytesPerSample = 2;
  const dataSize = frames * bytesPerSample;
  const buffer = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buffer);

  const writeAscii = (offset: number, text: string): void => {
    for (let i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i));
  };

  writeAscii(0, 'RIFF');
  view.setUint32(4, 36 + dataSize, true);
  writeAscii(8, 'WAVE');
  writeAscii(12, 'fmt ');
  view.setUint32(16, 16, true); // PCM fmt chunk size
  view.setUint16(20, 1, true); // audio format = PCM
  view.setUint16(22, 1, true); // channels = mono
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * bytesPerSample, true); // byte rate
  view.setUint16(32, bytesPerSample, true); // block align
  view.setUint16(34, 16, true); // bits per sample
  writeAscii(36, 'data');
  view.setUint32(40, dataSize, true);

  const amplitude = 0.25 * 0x7fff;
  for (let i = 0; i < frames; i++) {
    const sample = Math.sin((2 * Math.PI * frequencyHz * i) / sampleRate) * amplitude;
    view.setInt16(44 + i * bytesPerSample, sample, true);
  }
  return new Uint8Array(buffer);
}

/**
 * Create the sample playback source. `resolve()` synthesizes each track's tone
 * into an object URL and lays the tracks + straddling chapters onto the
 * book-absolute timeline. The `book` argument is ignored (the demo is synthetic).
 */
export function createSamplePlaybackSource(): PlaybackSource {
  return {
    id: SAMPLE_SOURCE_ID,
    async resolve(_book: Book): Promise<PlaybackManifest> {
      const layout = sampleManifestLayout();
      const tracks = layout.tracks.map((spec) => {
        const wav = encodeToneWav(spec.durationSeconds, spec.frequencyHz);
        const url = URL.createObjectURL(new Blob([wav.slice().buffer], { type: 'audio/wav' }));
        return { url, durationSeconds: spec.durationSeconds, mimeType: 'audio/wav' };
      });
      const timeline = assembleTimeline(tracks, layout.chapters);
      return { tracks: timeline.tracks, chapters: timeline.chapters };
    },
  };
}
