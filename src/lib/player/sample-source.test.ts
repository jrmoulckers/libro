import { describe, expect, it } from 'vitest';
import { encodeToneWav, sampleManifestLayout } from './sample-source';

function ascii(bytes: Uint8Array, offset: number, length: number): string {
  return String.fromCharCode(...bytes.slice(offset, offset + length));
}

describe('encodeToneWav', () => {
  it('writes a valid RIFF/WAVE header with the right data size', () => {
    const sampleRate = 8000;
    const seconds = 2;
    const wav = encodeToneWav(seconds, 220, sampleRate);

    const dataBytes = seconds * sampleRate * 2; // 16-bit mono
    expect(wav.byteLength).toBe(44 + dataBytes);
    expect(ascii(wav, 0, 4)).toBe('RIFF');
    expect(ascii(wav, 8, 4)).toBe('WAVE');
    expect(ascii(wav, 36, 4)).toBe('data');

    const view = new DataView(wav.buffer);
    expect(view.getUint32(40, true)).toBe(dataBytes); // data chunk size
    expect(view.getUint16(22, true)).toBe(1); // mono
    expect(view.getUint32(24, true)).toBe(sampleRate);
    expect(view.getUint16(34, true)).toBe(16); // bits per sample
  });

  it('produces a small clip and tolerates a zero-length duration', () => {
    expect(encodeToneWav(0, 220).byteLength).toBe(44);
    // A few seconds at 8 kHz mono stays well under ~50 KB.
    expect(encodeToneWav(6, 330).byteLength).toBeLessThan(100_000);
  });
});

describe('sampleManifestLayout', () => {
  it('has multi-track boundaries with chapters that straddle them', () => {
    const { tracks, chapters } = sampleManifestLayout();
    expect(tracks).toHaveLength(3);

    // Cumulative boundaries at 8 and 18.
    const boundaries = [
      tracks[0]!.durationSeconds,
      tracks[0]!.durationSeconds + tracks[1]!.durationSeconds,
    ];
    expect(boundaries).toEqual([8, 18]);

    // Chapter 2 spans the 8s boundary; chapter 3 spans the 18s boundary.
    const ch2 = chapters[1];
    const ch3 = chapters[2];
    expect(ch2!.startAbsolute < 8 && ch2!.endAbsolute > 8).toBe(true);
    expect(ch3!.startAbsolute < 18 && ch3!.endAbsolute > 18).toBe(true);
  });
});
