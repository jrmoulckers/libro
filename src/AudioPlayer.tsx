import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { AudioChapter, Book, PlaybackTrack, Progress } from "./types";
import { endOfChapterAbsolute, fadeMultiplier, locateAbsolute, mediaMetadataInput, nextChapterStart, positionStateInput, prevChapterStart, sleepRemainingSeconds, toAbsolute, totalDuration } from "./audioTimeline";
import { tryInvoke } from "./tauri";
import "./AudioPlayer.css";

/**
 * The audio the player should render: the ordered list of tracks (one per source
 * file) that make up one audiobook, laid out on a book-absolute timeline. This
 * is intentionally source-agnostic — the same component powers the bundled
 * public-domain browser demo (multiple `/sample-audiobook-N.wav` segments) and
 * an Audiobookshelf manifest resolved by `get_audiobook_stream` — so ABS and the
 * sample reuse one rendering path. A single-file book is just a one-track list.
 */
export interface AudioPlayerProps {
  /** Ordered tracks with cumulative `start_offset_seconds`. */
  tracks: PlaybackTrack[];
  /** Total book duration in seconds; defaults to the sum of track durations. */
  totalDuration?: number;
  /** Optional chapter markers (book-absolute times) for the jump-to list. */
  chapters?: AudioChapter[];
  /** Title, shown in the header. */
  title?: string;
  /**
   * Stable book id. When set (and under Tauri), the player restores and
   * persists listening progress via `get_listening_progress` /
   * `save_listening_progress`. Omit for the anonymous sample demo.
   */
  bookId?: string;
  /**
   * The full catalog book. When it came from Audiobookshelf and ABS
   * listening-sync is opted in, the backend best-effort mirrors the local
   * position back to the server (analogous to the Hardcover reading sync).
   */
  book?: Book;
  /** Seconds to skip on the back/forward buttons (default 30). */
  skipSeconds?: number;
  /** Close the player and return to the library. */
  onClose?: () => void;
}

const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];
/** Persist at most this often during playback (plus on pause / chapter change). */
const SAVE_INTERVAL_MS = 5000;
/** Sleep-timer countdown presets, in minutes. */
const SLEEP_PRESETS = [15, 30, 45, 60];
/** Linear volume fade over the final seconds of a timed sleep countdown. */
const SLEEP_FADE_SECONDS = 5;

/**
 * An armed in-app sleep timer. `duration` counts down to a wall-clock `expiresAt`
 * (with a volume fade near the end); `chapter` pauses when playback reaches the
 * book-absolute `targetAbsolute` (the end of the chapter that was playing when it
 * was armed).
 */
type SleepTimer =
  | { kind: "duration"; expiresAt: number }
  | { kind: "chapter"; targetAbsolute: number };

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const s = Math.floor(seconds % 60);
  const m = Math.floor((seconds / 60) % 60);
  const h = Math.floor(seconds / 3600);
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/**
 * In-app audiobook player (multi-track gapless playback).
 *
 * A webview HTML5-audio player that treats a multi-file ABS audiobook as ONE
 * continuous book on a **unified timeline**: play/pause, a scrubbable seek bar,
 * skip back/forward (default 30s), a playback-speed control (0.5×–3×), a chapter
 * list with jump-to-chapter, and a current-time / duration / percent readout —
 * all in **book-absolute** seconds. When a track ends it auto-advances to the
 * next (preserving speed); the next track is preloaded so the boundary gap is
 * minimal. Seek / skip / chapter-jump all cross track boundaries. Listening
 * position is persisted per-book (throttled) as whole-book seconds + fraction, so
 * the user resumes where they left off.
 *
 * Keyboard: space toggles play/pause, ←/→ skip, Esc closes.
 *
 * A **sleep timer** (in-app) can pause playback after 15/30/45/60 minutes — with
 * a short volume fade in the final seconds — or at the **end of the current
 * chapter**; it can be cancelled or extended (+5 min), and is cleared on a manual
 * pause/close so it can't fire later. Position is never reset: the throttled save
 * + resume handle it.
 *
 * OS now-playing / lockscreen / media keys are wired via the standard
 * **Media Session API** (`navigator.mediaSession`): metadata, book-absolute
 * `setPositionState`, and play/pause/seek/seek-to plus previous/next-chapter
 * handlers all drive the unified timeline. This lights up the OS transport where
 * the shell surfaces it (Windows SMTC / macOS Now Playing / Linux MPRIS / mobile
 * lockscreen); the plain webview build can't render the OS card itself.
 *
 * Deliberately NOT here (native/advanced work — tracked as TODOs): true
 * sample-accurate WebAudio gapless (this uses preloaded `<audio>` auto-advance),
 * variable-speed pitch correction, **true background playback / background sleep
 * timer** (needs native audio-session + foreground-service work — see
 * ARCHITECTURE.md), Android Auto / CarPlay, Chromecast, and an equalizer.
 */
export function AudioPlayer({
  tracks,
  totalDuration: totalProp,
  chapters = [],
  title,
  bookId,
  book,
  skipSeconds = 30,
  onClose,
}: AudioPlayerProps) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [trackIndex, setTrackIndex] = useState(0);
  const [within, setWithin] = useState(0);
  const [speed, setSpeed] = useState(1);
  const [restored, setRestored] = useState(bookId ? false : true);

  // In-app sleep timer (null = disarmed). `sleepRemaining` drives the countdown
  // readout; `sleepRef` mirrors the armed timer so stable handlers can read it.
  const [sleep, setSleep] = useState<SleepTimer | null>(null);
  const [sleepRemaining, setSleepRemaining] = useState(0);
  const sleepRef = useRef<SleepTimer | null>(null);
  useEffect(() => {
    sleepRef.current = sleep;
  }, [sleep]);

  const lastSaveRef = useRef(0);
  // Within-track offset to apply once the (newly-switched) track's metadata is
  // ready — seeking requires the media to be loaded first.
  const pendingWithinRef = useRef<number | null>(null);
  // Whether to auto-resume playback after a track switch (auto-advance / seek
  // while playing).
  const resumePlayRef = useRef(false);
  // Latest values mirrored into refs so event handlers stay stable.
  const playingRef = useRef(false);
  const speedRef = useRef(1);

  const total = useMemo(
    () => (totalProp && totalProp > 0 ? totalProp : totalDuration(tracks)),
    [totalProp, tracks],
  );
  const currentTrack = tracks[trackIndex];
  const nextTrack = tracks[trackIndex + 1];
  // The unified, book-absolute position the whole UI speaks in.
  const absolute = toAbsolute(tracks, trackIndex, within);
  // Mirrored into a ref so the (stable) OS media-key handlers can read the live
  // position without being re-registered on every tick.
  const absoluteRef = useRef(0);
  useEffect(() => {
    absoluteRef.current = absolute;
  }, [absolute]);

  // Restore any saved listening position (book-absolute) before playback starts,
  // mapping it back to the right track + within-track offset.
  useEffect(() => {
    let cancelled = false;
    if (!bookId) return;
    void (async () => {
      const p = await tryInvoke<Progress | null>("get_listening_progress", {
        bookId,
      });
      if (cancelled) return;
      if (p?.position_seconds != null && p.position_seconds > 0) {
        const { index, offset } = locateAbsolute(tracks, p.position_seconds);
        setTrackIndex(index);
        setWithin(offset);
        pendingWithinRef.current = offset;
      }
      setRestored(true);
    })();
    return () => {
      cancelled = true;
    };
    // Intentionally keyed only on bookId: restore runs once per book.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bookId]);

  const persist = useCallback(
    (absolutePos: number, finished: boolean) => {
      if (!bookId) return;
      const fraction = total > 0 ? Math.min(1, absolutePos / total) : 0;
      const progress: Progress = {
        fraction,
        position_seconds: absolutePos,
        locator: null,
        finished: finished || fraction >= 0.999,
      };
      // `book` lets the backend best-effort mirror the whole-book position to
      // Audiobookshelf (opt-in; only for ABS-sourced books). The number is now
      // book-absolute, which is exactly what the ABS progress API expects.
      void tryInvoke("save_listening_progress", { bookId, progress, book });
    },
    [bookId, book, total],
  );

  // Switch the active track, remembering the within-track offset to seek to once
  // it loads and whether to resume playing at the boundary.
  const switchToTrack = useCallback(
    (index: number, offset: number, autoplay: boolean) => {
      pendingWithinRef.current = offset;
      resumePlayRef.current = autoplay;
      setWithin(offset);
      setTrackIndex(index);
    },
    [],
  );

  // Seek to a book-absolute position, switching tracks if it lands in another.
  const seekAbsolute = useCallback(
    (absoluteTarget: number, save = false) => {
      const clamped = Math.max(0, Math.min(absoluteTarget, total));
      const { index, offset } = locateAbsolute(tracks, clamped);
      if (index === trackIndex) {
        const el = audioRef.current;
        if (el) el.currentTime = offset;
        setWithin(offset);
      } else {
        switchToTrack(index, offset, playingRef.current);
      }
      if (save) persist(clamped, false);
    },
    [tracks, total, trackIndex, switchToTrack, persist],
  );

  const skip = useCallback(
    (delta: number) => seekAbsolute(absolute + delta),
    [seekAbsolute, absolute],
  );

  const playAudio = useCallback(() => {
    void audioRef.current?.play();
  }, []);

  // Pause and disarm any armed sleep timer (so it can't fire later), restoring
  // full volume. Shared by the button, the keyboard, and the OS media controls.
  const pauseAudio = useCallback(() => {
    const el = audioRef.current;
    if (!el) return;
    el.pause();
    if (sleepRef.current) {
      setSleep(null);
      el.volume = 1;
    }
  }, []);

  const togglePlay = useCallback(() => {
    const el = audioRef.current;
    if (!el) return;
    if (el.paused) {
      playAudio();
    } else {
      pauseAudio();
    }
  }, [playAudio, pauseAudio]);

  const changeSpeed = useCallback((rate: number) => {
    const el = audioRef.current;
    if (el) el.playbackRate = rate;
    speedRef.current = rate;
    setSpeed(rate);
  }, []);

  // --- Sleep timer --------------------------------------------------------

  const restoreVolume = useCallback(() => {
    const el = audioRef.current;
    if (el) el.volume = 1;
  }, []);

  const armDurationSleep = useCallback((minutes: number) => {
    setSleep({ kind: "duration", expiresAt: Date.now() + minutes * 60_000 });
    setSleepRemaining(minutes * 60);
  }, []);

  const armChapterSleep = useCallback(() => {
    const target = endOfChapterAbsolute(chapters, toAbsolute(tracks, trackIndex, within));
    if (target == null) return; // no chapter to end on
    setSleep({ kind: "chapter", targetAbsolute: target });
  }, [chapters, tracks, trackIndex, within]);

  const cancelSleep = useCallback(() => {
    setSleep(null);
    restoreVolume();
  }, [restoreVolume]);

  // "+5 min" still-awake extend (duration timers only). Also un-fades.
  const extendSleep = useCallback(
    (minutes: number) => {
      setSleep((s) =>
        s && s.kind === "duration"
          ? { kind: "duration", expiresAt: s.expiresAt + minutes * 60_000 }
          : s,
      );
      restoreVolume();
    },
    [restoreVolume],
  );

  // Duration countdown: tick the remaining time, fade the volume near the end,
  // and pause (without resetting position) on expiry.
  useEffect(() => {
    if (!sleep || sleep.kind !== "duration") return;
    const tick = () => {
      const remaining = sleepRemainingSeconds(sleep.expiresAt, Date.now());
      setSleepRemaining(remaining);
      const el = audioRef.current;
      if (el) el.volume = fadeMultiplier(remaining, SLEEP_FADE_SECONDS);
      if (remaining <= 0) {
        if (el) {
          el.pause();
          el.volume = 1;
        }
        setSleep(null);
      }
    };
    tick();
    const id = window.setInterval(tick, 500);
    return () => window.clearInterval(id);
  }, [sleep]);

  // End-of-chapter timer: pause once the unified position reaches the target.
  useEffect(() => {
    if (!sleep || sleep.kind !== "chapter") return;
    if (absolute >= sleep.targetAbsolute) {
      audioRef.current?.pause();
      setSleep(null);
    }
  }, [sleep, absolute]);

  // Throttled progress save: at most every SAVE_INTERVAL_MS during playback.
  const handleTimeUpdate = useCallback(() => {
    const el = audioRef.current;
    if (!el) return;
    setWithin(el.currentTime);
    const now = Date.now();
    if (now - lastSaveRef.current >= SAVE_INTERVAL_MS) {
      lastSaveRef.current = now;
      persist(toAbsolute(tracks, trackIndex, el.currentTime), el.ended);
    }
  }, [persist, tracks, trackIndex]);

  const handleLoadedMetadata = useCallback(() => {
    const el = audioRef.current;
    if (!el) return;
    // Re-apply the chosen speed — a fresh track resets playbackRate to 1.
    el.playbackRate = speedRef.current;
    // Apply a pending within-track seek (restore or cross-boundary switch).
    if (pendingWithinRef.current != null) {
      try {
        el.currentTime = pendingWithinRef.current;
      } catch {
        /* seeking may be unavailable until further buffering — non-fatal */
      }
      setWithin(pendingWithinRef.current);
      pendingWithinRef.current = null;
    }
    if (resumePlayRef.current) {
      resumePlayRef.current = false;
      void el.play();
    }
  }, []);

  // A track finished: auto-advance to the next (gapless-ish, preserving speed),
  // or mark the book finished at the very end.
  const handleEnded = useCallback(() => {
    if (trackIndex < tracks.length - 1) {
      switchToTrack(trackIndex + 1, 0, true);
    } else {
      setPlaying(false);
      persist(total, true);
    }
  }, [trackIndex, tracks.length, switchToTrack, persist, total]);

  // Save on pause so a position isn't lost between throttle ticks.
  const handlePause = useCallback(() => {
    setPlaying(false);
    playingRef.current = false;
    const el = audioRef.current;
    if (el) persist(toAbsolute(tracks, trackIndex, el.currentTime), el.ended);
  }, [persist, tracks, trackIndex]);

  const handlePlay = useCallback(() => {
    setPlaying(true);
    playingRef.current = true;
  }, []);

  // Keyboard: space = play/pause, arrows = skip, Esc = close.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) {
        return;
      }
      if (e.key === " " || e.code === "Space") {
        e.preventDefault();
        togglePlay();
      } else if (e.key === "ArrowLeft") {
        skip(-skipSeconds);
      } else if (e.key === "ArrowRight") {
        skip(skipSeconds);
      } else if (e.key === "Escape") {
        onClose?.();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [togglePlay, skip, skipSeconds, onClose]);

  // Jump to the next chapter on the unified timeline (media "nexttrack"); fall
  // back to a plain skip-forward when there are no chapters to jump to.
  const gotoNextChapter = useCallback(() => {
    const target = nextChapterStart(chapters, absoluteRef.current);
    if (target != null) {
      seekAbsolute(target, true);
    } else {
      skip(skipSeconds);
    }
  }, [chapters, seekAbsolute, skip, skipSeconds]);

  // Jump to the previous chapter (media "previoustrack") — restart the current
  // chapter or step to the prior one; fall back to skip-back with no chapters.
  const gotoPrevChapter = useCallback(() => {
    const target = prevChapterStart(chapters, absoluteRef.current);
    if (target != null) {
      seekAbsolute(target, true);
    } else {
      skip(-skipSeconds);
    }
  }, [chapters, seekAbsolute, skip, skipSeconds]);

  // --- OS now-playing / lockscreen / media keys (Media Session API) --------
  //
  // Wires the standard `navigator.mediaSession` to our unified multi-track
  // player so the OS now-playing card, lockscreen transport, and hardware /
  // keyboard media keys drive whole-book playback. Feature-detected so browsers
  // / webviews without the API simply no-op. Actual OS surfacing (Windows SMTC /
  // macOS Now Playing / Linux MPRIS / mobile lockscreen) requires the real
  // desktop/mobile shell — the plain webview here registers the handlers but
  // can't display the OS card.

  // Now-playing metadata: refreshed whenever the book (or title) changes.
  useEffect(() => {
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
    const input = mediaMetadataInput(book, title);
    try {
      navigator.mediaSession.metadata = new MediaMetadata({
        title: input.title,
        artist: input.artist,
        album: input.album,
        artwork: input.artwork,
      });
    } catch {
      /* MediaMetadata unavailable in this webview — non-fatal */
    }
  }, [book, title]);

  // Playback state + position on the book-absolute timeline, so the OS scrubber
  // reflects whole-book progress (not the current file). Runs as `absolute`
  // ticks during playback.
  useEffect(() => {
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
    const ms = navigator.mediaSession;
    ms.playbackState = playing ? "playing" : "paused";
    const ps = positionStateInput(total, absolute, speed);
    if (ps && typeof ms.setPositionState === "function") {
      try {
        ms.setPositionState(ps);
      } catch {
        /* invalid duration/position for this tick — skip */
      }
    }
  }, [playing, absolute, total, speed]);

  // Transport action handlers. seekto maps the OS scrubber's absolute target
  // through our existing locateAbsolute→(track,offset) seek; previous/next map
  // to chapter navigation across track boundaries. Cleaned up on unmount.
  useEffect(() => {
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
    const ms = navigator.mediaSession;
    const set = (
      action: MediaSessionAction,
      handler: MediaSessionActionHandler | null,
    ) => {
      try {
        ms.setActionHandler(action, handler);
      } catch {
        /* some actions are unsupported in a given webview — ignore */
      }
    };
    set("play", () => playAudio());
    set("pause", () => pauseAudio());
    set("seekbackward", (d) => skip(-(d.seekOffset ?? skipSeconds)));
    set("seekforward", (d) => skip(d.seekOffset ?? skipSeconds));
    set("seekto", (d) => {
      if (typeof d.seekTime === "number") seekAbsolute(d.seekTime, true);
    });
    set("previoustrack", () => gotoPrevChapter());
    set("nexttrack", () => gotoNextChapter());
    return () => {
      for (const action of [
        "play",
        "pause",
        "seekbackward",
        "seekforward",
        "seekto",
        "previoustrack",
        "nexttrack",
      ] as MediaSessionAction[]) {
        set(action, null);
      }
    };
  }, [playAudio, pauseAudio, skip, skipSeconds, seekAbsolute, gotoPrevChapter, gotoNextChapter]);

  const activeChapterIndex = useMemo(() => {
    if (chapters.length === 0) return -1;
    return chapters.findIndex((c) => absolute >= c.start && absolute < c.end);
  }, [chapters, absolute]);

  const jumpToChapter = useCallback(
    (chapter: AudioChapter) => {
      seekAbsolute(chapter.start, true);
      const el = audioRef.current;
      const { index } = locateAbsolute(tracks, chapter.start);
      if (index === trackIndex) {
        // Same track: play immediately.
        if (el && el.paused) void el.play();
      } else {
        // Different track: resume once the new track's metadata loads.
        resumePlayRef.current = true;
      }
    },
    [seekAbsolute, tracks, trackIndex],
  );

  const percent = total > 0 ? Math.round((absolute / total) * 100) : 0;

  return (
    <div className="player">
      <header className="player__bar">
        <button className="player__close" onClick={() => onClose?.()}>
          ← Library
        </button>
        <span className="player__title" title={title}>
          {title ?? "Now playing"}
        </span>
        <span className="player__percent">{percent}%</span>
      </header>

      <div className="player__stage">
        {restored && currentTrack && (
          <audio
            ref={audioRef}
            src={currentTrack.url}
            preload="metadata"
            onLoadedMetadata={handleLoadedMetadata}
            onTimeUpdate={handleTimeUpdate}
            onPlay={handlePlay}
            onPause={handlePause}
            onEnded={handleEnded}
          />
        )}
        {/* Preload the upcoming track so the auto-advance boundary gap is small.
            True sample-accurate gapless needs WebAudio (documented TODO). */}
        {nextTrack && (
          <audio
            key={nextTrack.url}
            src={nextTrack.url}
            preload="auto"
            style={{ display: "none" }}
          />
        )}

        {tracks.length > 1 && (
          <p className="player__track-indicator">
            Track {trackIndex + 1} of {tracks.length}
          </p>
        )}

        <div className="player__seek">
          <span className="player__time">{formatTime(absolute)}</span>
          <input
            className="player__scrub"
            type="range"
            min={0}
            max={Math.max(0, total)}
            step={0.1}
            value={Math.min(absolute, total || absolute)}
            aria-label="Seek"
            onChange={(e) => seekAbsolute(Number(e.target.value))}
          />
          <span className="player__time">{formatTime(total)}</span>
        </div>

        <div className="player__controls">
          <button aria-label="Skip back" onClick={() => skip(-skipSeconds)}>
            ⟲ {skipSeconds}s
          </button>
          <button className="player__play" aria-label="Play/pause" onClick={togglePlay}>
            {playing ? "❚❚ Pause" : "► Play"}
          </button>
          <button aria-label="Skip forward" onClick={() => skip(skipSeconds)}>
            {skipSeconds}s ⟳
          </button>

          <label className="player__speed">
            Speed
            <select
              value={speed}
              onChange={(e) => changeSpeed(Number(e.target.value))}
            >
              {SPEEDS.map((s) => (
                <option key={s} value={s}>
                  {s}×
                </option>
              ))}
            </select>
          </label>
        </div>

        <div className="player__sleep">
          <span className="player__sleep-label">Sleep timer</span>
          {sleep === null ? (
            <div className="player__sleep-options">
              {SLEEP_PRESETS.map((m) => (
                <button key={m} onClick={() => armDurationSleep(m)}>
                  {m}m
                </button>
              ))}
              {chapters.length > 0 && (
                <button onClick={armChapterSleep}>End of chapter</button>
              )}
            </div>
          ) : (
            <div className="player__sleep-armed">
              <span className="player__sleep-remaining">
                {sleep.kind === "duration"
                  ? `Pausing in ${formatTime(sleepRemaining)}`
                  : "Pausing at end of chapter"}
              </span>
              {sleep.kind === "duration" && (
                <button onClick={() => extendSleep(5)}>+5 min</button>
              )}
              <button onClick={cancelSleep}>Cancel</button>
            </div>
          )}
        </div>

        {chapters.length > 0 && (
          <div className="player__chapters">
            <h2>Chapters</h2>
            <ol>
              {chapters.map((c, i) => (
                <li key={c.id}>
                  <button
                    className={i === activeChapterIndex ? "is-active" : ""}
                    onClick={() => jumpToChapter(c)}
                  >
                    <span className="player__chapter-title">{c.title}</span>
                    <span className="player__chapter-time">
                      {formatTime(c.start)}
                    </span>
                  </button>
                </li>
              ))}
            </ol>
          </div>
        )}

        <p className="player__hint">
          Space = play/pause · ← / → = skip {skipSeconds}s
        </p>
      </div>
    </div>
  );
}

export default AudioPlayer;
