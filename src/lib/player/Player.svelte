<script lang="ts">
  /**
   * In-app audiobook player — a lazily-mounted modal over the library.
   *
   * A single native HTML5 `<audio>` element renders a multi-file audiobook as ONE
   * continuous **book-absolute** timeline: play/pause, a whole-book scrubber, skip
   * ±N seconds, chapter prev/next + jump-to-chapter, a playback-rate control, and a
   * sleep timer (15/45/… min presets + "end of chapter", with a volume fade and
   * "+5 min" extend). When a track ends it auto-advances to the next (position never
   * resets); seek / skip / chapter-jump cross track boundaries via the pure
   * {@link ./timeline} math. Listening position is restored on open and persisted
   * (throttled) to the {@link ./listening-store}.
   *
   * OS now-playing / lockscreen / media keys are wired through the standard
   * `navigator.mediaSession` (feature-detected; a no-op where absent), fed the
   * book-ABSOLUTE position so the OS scrubber reflects whole-book progress.
   *
   * This component is the thin browser shell; all timeline/sleep/chapter/media math
   * lives in pure, unit-tested modules. It is dynamically imported from `App.svelte`
   * so it (and any source code it pulls) stays out of the main entry chunk.
   */
  import { onDestroy, onMount } from 'svelte';
  import type { Book } from '../models';
  import type { ListeningStore } from './listening-store';
  import type { PlaybackSource } from './source';
  import {
    assembleTimeline,
    endOfChapterAbsolute,
    fadeMultiplier,
    listeningProgress,
    locateAbsolute,
    mediaMetadataInput,
    nextChapterStart,
    positionStateInput,
    prevChapterStart,
    sleepRemainingSeconds,
    trackToAbsolute,
    type Chapter,
    type Timeline,
  } from './timeline';

  interface Props {
    book: Book;
    source: PlaybackSource;
    listeningStore: ListeningStore;
    onClose: () => void;
    /** Seconds to skip on the back/forward buttons. */
    skipSeconds?: number;
  }

  const { book, source, listeningStore, onClose, skipSeconds = 30 }: Props = $props();

  type Status = 'loading' | 'ready' | 'error';
  type SleepTimer =
    { kind: 'duration'; expiresAt: number } | { kind: 'chapter'; targetAbsolute: number };

  const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];
  const SLEEP_PRESETS = [15, 30, 45, 60];
  const SLEEP_FADE_SECONDS = 5;
  const SAVE_INTERVAL_MS = 5000;

  let status = $state<Status>('loading');
  let tl = $state<Timeline | null>(null);
  let trackIndex = $state(0);
  let within = $state(0);
  let speed = $state(1);
  let playing = $state(false);
  let sleep = $state<SleepTimer | null>(null);
  let sleepRemaining = $state(0);

  let audioEl = $state<HTMLAudioElement>();
  let closeButton = $state<HTMLButtonElement>();

  // Non-reactive coordination between a track switch and its async metadata load.
  let pendingWithin: number | null = null;
  let resumeOnLoad = false;
  let lastSave = 0;

  const tracks = $derived(tl?.tracks ?? []);
  const chapters = $derived<Chapter[]>(tl?.chapters ?? []);
  const total = $derived(tl?.totalDuration ?? 0);
  const absolute = $derived(trackToAbsolute(tracks, trackIndex, within));
  const currentSrc = $derived(tracks[trackIndex]?.url ?? '');
  const percent = $derived(total > 0 ? Math.round((absolute / total) * 100) : 0);
  const activeChapterIndex = $derived(
    chapters.findIndex((c) => absolute >= c.startAbsolute && absolute < c.endAbsolute),
  );

  function formatTime(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return '0:00';
    const s = Math.floor(seconds % 60);
    const m = Math.floor((seconds / 60) % 60);
    const h = Math.floor(seconds / 3600);
    const ss = String(s).padStart(2, '0');
    return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
  }

  // --- persistence --------------------------------------------------------
  async function persist(absolutePos: number, ended: boolean): Promise<void> {
    const progress = listeningProgress(absolutePos, total, ended);
    await listeningStore.set(book.id, {
      positionSeconds: progress.positionSeconds ?? absolutePos,
      fraction: progress.fraction,
      finished: progress.finished,
    });
  }

  // --- track / seek control ----------------------------------------------
  function switchToTrack(index: number, offset: number, autoplay: boolean): void {
    pendingWithin = offset;
    resumeOnLoad = autoplay;
    within = offset;
    trackIndex = index;
  }

  function seekAbsolute(target: number, save = false): void {
    const clamped = Math.max(0, Math.min(target, total));
    const { trackIndex: index, offsetInTrack } = locateAbsolute(tracks, clamped);
    if (index === trackIndex) {
      if (audioEl) audioEl.currentTime = offsetInTrack;
      within = offsetInTrack;
    } else {
      switchToTrack(index, offsetInTrack, playing);
    }
    if (save) void persist(clamped, false);
  }

  function skip(delta: number): void {
    seekAbsolute(absolute + delta, true);
  }

  function onScrub(event: Event): void {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    seekAbsolute(value, true);
  }

  // --- play / pause / speed ----------------------------------------------
  function playAudio(): void {
    void audioEl?.play();
  }

  function pauseAudio(): void {
    audioEl?.pause();
    if (sleep) {
      sleep = null;
      if (audioEl) audioEl.volume = 1;
    }
  }

  function togglePlay(): void {
    if (!audioEl) return;
    if (audioEl.paused) playAudio();
    else pauseAudio();
  }

  function changeSpeed(event: Event): void {
    const rate = Number((event.currentTarget as HTMLSelectElement).value);
    if (audioEl) audioEl.playbackRate = rate;
    speed = rate;
  }

  // --- chapter navigation -------------------------------------------------
  function gotoNextChapter(): void {
    const target = nextChapterStart(chapters, absolute);
    if (target != null) seekAbsolute(target, true);
    else skip(skipSeconds);
  }

  function gotoPrevChapter(): void {
    const target = prevChapterStart(chapters, absolute);
    if (target != null) seekAbsolute(target, true);
    else skip(-skipSeconds);
  }

  function jumpToChapter(chapter: Chapter): void {
    const wasPlaying = playing;
    seekAbsolute(chapter.startAbsolute, true);
    if (wasPlaying) resumeOnLoad = true;
    else playAudio();
  }

  // --- sleep timer --------------------------------------------------------
  function armDurationSleep(minutes: number): void {
    sleep = { kind: 'duration', expiresAt: Date.now() + minutes * 60_000 };
    sleepRemaining = minutes * 60;
  }

  function armChapterSleep(): void {
    const target = endOfChapterAbsolute(chapters, absolute);
    if (target != null) sleep = { kind: 'chapter', targetAbsolute: target };
  }

  function cancelSleep(): void {
    sleep = null;
    if (audioEl) audioEl.volume = 1;
  }

  function extendSleep(minutes: number): void {
    if (sleep?.kind === 'duration') {
      sleep = { kind: 'duration', expiresAt: sleep.expiresAt + minutes * 60_000 };
    }
    if (audioEl) audioEl.volume = 1;
  }

  // Duration countdown: tick remaining, fade near the end, pause on expiry.
  $effect(() => {
    if (sleep?.kind !== 'duration') return;
    const expiresAt = sleep.expiresAt;
    const tick = (): void => {
      const remaining = sleepRemainingSeconds(expiresAt, Date.now());
      sleepRemaining = remaining;
      if (audioEl) audioEl.volume = fadeMultiplier(remaining, SLEEP_FADE_SECONDS);
      if (remaining <= 0) {
        if (audioEl) {
          audioEl.pause();
          audioEl.volume = 1;
        }
        sleep = null;
      }
    };
    tick();
    const id = window.setInterval(tick, 500);
    return () => window.clearInterval(id);
  });

  // End-of-chapter timer: pause once the unified position reaches the target.
  $effect(() => {
    if (sleep?.kind === 'chapter' && absolute >= sleep.targetAbsolute) {
      audioEl?.pause();
      sleep = null;
    }
  });

  // --- audio element events ----------------------------------------------
  function handleLoadedMetadata(): void {
    if (!audioEl) return;
    audioEl.playbackRate = speed; // a fresh track resets rate to 1
    if (pendingWithin != null) {
      try {
        audioEl.currentTime = pendingWithin;
      } catch {
        /* seeking may need more buffering — non-fatal */
      }
      within = pendingWithin;
      pendingWithin = null;
    }
    if (resumeOnLoad) {
      resumeOnLoad = false;
      void audioEl.play();
    }
  }

  function handleTimeUpdate(): void {
    if (!audioEl) return;
    within = audioEl.currentTime;
    const now = Date.now();
    if (now - lastSave >= SAVE_INTERVAL_MS) {
      lastSave = now;
      void persist(trackToAbsolute(tracks, trackIndex, audioEl.currentTime), audioEl.ended);
    }
  }

  function handleEnded(): void {
    if (trackIndex < tracks.length - 1) {
      switchToTrack(trackIndex + 1, 0, true);
    } else {
      playing = false;
      void persist(total, true);
    }
  }

  function handlePlay(): void {
    playing = true;
  }

  function handlePause(): void {
    playing = false;
    if (audioEl)
      void persist(trackToAbsolute(tracks, trackIndex, audioEl.currentTime), audioEl.ended);
  }

  // --- keyboard -----------------------------------------------------------
  function handleKey(event: KeyboardEvent): void {
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === 'INPUT' || target.tagName === 'SELECT')) return;
    if (event.key === ' ' || event.code === 'Space') {
      event.preventDefault();
      togglePlay();
    } else if (event.key === 'ArrowLeft') {
      skip(-skipSeconds);
    } else if (event.key === 'ArrowRight') {
      skip(skipSeconds);
    } else if (event.key === 'Escape') {
      onClose();
    }
  }

  // --- Media Session (feature-detected) -----------------------------------
  const hasMediaSession = typeof navigator !== 'undefined' && 'mediaSession' in navigator;

  // Now-playing metadata, refreshed when the book changes.
  $effect(() => {
    if (!hasMediaSession) return;
    const input = mediaMetadataInput(book);
    try {
      navigator.mediaSession.metadata = new MediaMetadata(input);
    } catch {
      /* MediaMetadata unavailable in this environment — non-fatal */
    }
  });

  // Playback state + book-absolute position for the OS scrubber.
  $effect(() => {
    if (!hasMediaSession) return;
    const ms = navigator.mediaSession;
    ms.playbackState = playing ? 'playing' : 'paused';
    const ps = positionStateInput(total, absolute, speed);
    if (ps && typeof ms.setPositionState === 'function') {
      try {
        ms.setPositionState(ps);
      } catch {
        /* invalid duration/position this tick — skip */
      }
    }
  });

  function setMediaHandlers(active: boolean): void {
    if (!hasMediaSession) return;
    const ms = navigator.mediaSession;
    type Action = Parameters<MediaSession['setActionHandler']>[0];
    type Handler = Parameters<MediaSession['setActionHandler']>[1];
    const set = (action: Action, handler: Handler): void => {
      try {
        ms.setActionHandler(action, handler);
      } catch {
        /* unsupported action in this environment — ignore */
      }
    };
    set('play', active ? () => playAudio() : null);
    set('pause', active ? () => pauseAudio() : null);
    set('seekbackward', active ? (d) => skip(-(d.seekOffset ?? skipSeconds)) : null);
    set('seekforward', active ? (d) => skip(d.seekOffset ?? skipSeconds) : null);
    set(
      'seekto',
      active
        ? (d) => {
            if (typeof d.seekTime === 'number') seekAbsolute(d.seekTime, true);
          }
        : null,
    );
    set('previoustrack', active ? () => gotoPrevChapter() : null);
    set('nexttrack', active ? () => gotoNextChapter() : null);
  }

  // --- lifecycle ----------------------------------------------------------
  onMount(async () => {
    closeButton?.focus();
    try {
      const manifest = await source.resolve(book);
      tl = assembleTimeline(manifest.tracks, manifest.chapters);

      const saved = await listeningStore.get(book.id);
      if (saved && saved.positionSeconds > 0) {
        const { trackIndex: index, offsetInTrack } = locateAbsolute(
          tl.tracks,
          saved.positionSeconds,
        );
        trackIndex = index;
        within = offsetInTrack;
        pendingWithin = offsetInTrack;
      }
      status = 'ready';
      setMediaHandlers(true);
    } catch {
      status = 'error';
    }
  });

  onDestroy(() => {
    setMediaHandlers(false);
    if (audioEl) {
      audioEl.pause();
      void persist(trackToAbsolute(tracks, trackIndex, audioEl.currentTime), audioEl.ended);
    }
    // Free the object URLs the sample source minted for this book.
    for (const track of tracks) {
      if (track.url.startsWith('blob:')) URL.revokeObjectURL(track.url);
    }
  });
</script>

<svelte:window onkeydown={handleKey} />

<div class="backdrop">
  <div class="player" role="dialog" aria-modal="true" aria-label={`Playing ${book.title}`}>
    <header class="bar">
      <div class="meta">
        <span class="book-title">{book.title}</span>
        {#if status === 'ready'}
          <span class="pos" aria-live="polite">
            {formatTime(absolute)} / {formatTime(total)} · {percent}%
          </span>
        {/if}
      </div>
      <button type="button" bind:this={closeButton} onclick={onClose}>Close</button>
    </header>

    {#if status === 'loading'}
      <p class="status" role="status">Loading audio…</p>
    {:else if status === 'error'}
      <p class="status" role="alert">This audiobook could not be played.</p>
    {:else}
      <!-- One native audio element renders the whole multi-track book. -->
      <audio
        bind:this={audioEl}
        src={currentSrc}
        preload="metadata"
        onloadedmetadata={handleLoadedMetadata}
        ontimeupdate={handleTimeUpdate}
        onended={handleEnded}
        onplay={handlePlay}
        onpause={handlePause}
      ></audio>

      <div class="transport">
        <button type="button" onclick={gotoPrevChapter} aria-label="Previous chapter">⏮</button>
        <button
          type="button"
          onclick={() => skip(-skipSeconds)}
          aria-label={`Back ${skipSeconds} seconds`}
        >
          ↺ {skipSeconds}
        </button>
        <button type="button" class="play" onclick={togglePlay}>
          {playing ? 'Pause' : 'Play'}
        </button>
        <button
          type="button"
          onclick={() => skip(skipSeconds)}
          aria-label={`Forward ${skipSeconds} seconds`}
        >
          ↻ {skipSeconds}
        </button>
        <button type="button" onclick={gotoNextChapter} aria-label="Next chapter">⏭</button>
      </div>

      <input
        class="scrubber"
        type="range"
        min="0"
        max={total}
        step="1"
        value={absolute}
        aria-label="Seek within book"
        oninput={onScrub}
      />

      <div class="options">
        <label>
          Speed
          <select value={speed} onchange={changeSpeed} aria-label="Playback speed">
            {#each SPEEDS as rate (rate)}
              <option value={rate}>{rate}×</option>
            {/each}
          </select>
        </label>

        <div class="sleep" aria-label="Sleep timer">
          {#if sleep}
            <span class="sleep-status" aria-live="polite">
              {#if sleep.kind === 'duration'}
                Sleep in {formatTime(sleepRemaining)}
              {:else}
                Sleep at end of chapter
              {/if}
            </span>
            {#if sleep.kind === 'duration'}
              <button type="button" onclick={() => extendSleep(5)}>+5 min</button>
            {/if}
            <button type="button" onclick={cancelSleep}>Cancel</button>
          {:else}
            {#each SLEEP_PRESETS as minutes (minutes)}
              <button type="button" onclick={() => armDurationSleep(minutes)}>{minutes}m</button>
            {/each}
            {#if chapters.length > 0}
              <button type="button" onclick={armChapterSleep}>End of chapter</button>
            {/if}
          {/if}
        </div>
      </div>

      {#if chapters.length > 0}
        <nav class="chapters" aria-label="Chapters">
          <ul>
            {#each chapters as chapter, i (chapter.startAbsolute + chapter.title)}
              <li>
                <button
                  type="button"
                  class:current={i === activeChapterIndex}
                  onclick={() => jumpToChapter(chapter)}
                >
                  <span class="ch-title">{chapter.title}</span>
                  <span class="ch-time">{formatTime(chapter.startAbsolute)}</span>
                </button>
              </li>
            {/each}
          </ul>
        </nav>
      {/if}
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.5);
    z-index: 100;
  }

  .player {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    width: min(40rem, 100%);
    max-height: 100%;
    padding: 1rem;
    overflow: auto;
  }

  .bar {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .book-title {
    font-weight: 600;
  }

  .pos {
    font-size: 0.875rem;
  }

  .transport {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
  }

  .scrubber {
    width: 100%;
  }

  .options {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }

  .sleep {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }

  .chapters {
    /* Reserve room so the list doesn't shift the dialog as it loads (no CLS). */
    min-height: 6rem;
    overflow: auto;
  }

  .chapters ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .chapters button {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 1rem;
    text-align: start;
    padding: 0.375rem 0.5rem;
  }

  .chapters button.current {
    font-weight: 700;
  }

  .status {
    margin: auto;
  }
</style>
