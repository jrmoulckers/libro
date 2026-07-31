import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { AudioChapter, Book, Progress } from "./types";
import { tryInvoke } from "./tauri";
import "./AudioPlayer.css";

/**
 * The audio the player should render: a URL an `<audio>` element can load. This
 * is intentionally source-agnostic — the same component powers the bundled
 * public-domain browser demo (`/sample-audiobook.wav`) and the Audiobookshelf
 * stream URL resolved by the `get_audiobook_stream` command — so ABS and the
 * sample reuse one rendering path.
 */
export interface AudioPlayerProps {
  /** Directly-playable audio URL. */
  src: string;
  /** Optional chapter markers for the jump-to-chapter list. */
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
 * In-app audiobook player (playback phase v1).
 *
 * A webview HTML5-audio player: play/pause, a scrubbable seek bar, skip
 * back/forward (default 30s), a playback-speed control (0.5×–3×), a chapter list
 * with jump-to-chapter, and a current-time / duration / percent readout.
 * Keyboard: space toggles play/pause, ←/→ skip. Listening position is persisted
 * per-book (throttled) so the user resumes where they left off.
 *
 * Deliberately NOT in v1 (native platform work — tracked as TODOs): background
 * playback, lockscreen / now-playing controls, Android Auto / CarPlay,
 * Chromecast, sleep timer, and an equalizer. Outward progress sync to
 * Audiobookshelf is also a later phase.
 */
export function AudioPlayer({
  src,
  chapters = [],
  title,
  bookId,
  book,
  skipSeconds = 30,
  onClose,
}: AudioPlayerProps) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [current, setCurrent] = useState(0);
  const [duration, setDuration] = useState(0);
  const [speed, setSpeed] = useState(1);
  const [restored, setRestored] = useState(bookId ? false : true);

  const lastSaveRef = useRef(0);
  const resumeToRef = useRef<number | null>(null);

  // Restore any saved listening position before playback starts.
  useEffect(() => {
    let cancelled = false;
    if (!bookId) return;
    void (async () => {
      const p = await tryInvoke<Progress | null>("get_listening_progress", {
        bookId,
      });
      if (cancelled) return;
      if (p?.position_seconds != null && p.position_seconds > 0) {
        resumeToRef.current = p.position_seconds;
      }
      setRestored(true);
    })();
    return () => {
      cancelled = true;
    };
  }, [bookId]);

  const persist = useCallback(
    (positionSeconds: number, total: number, finished: boolean) => {
      if (!bookId) return;
      const fraction = total > 0 ? Math.min(1, positionSeconds / total) : 0;
      const progress: Progress = {
        fraction,
        position_seconds: positionSeconds,
        locator: null,
        finished: finished || fraction >= 0.999,
      };
      // `book` lets the backend best-effort mirror the position to Audiobookshelf
      // (opt-in; only for ABS-sourced books). See libro_core::listening_sync.
      void tryInvoke("save_listening_progress", { bookId, progress, book });
    },
    [bookId, book],
  );

  // Throttled progress save: at most every SAVE_INTERVAL_MS during playback.
  const handleTimeUpdate = useCallback(() => {
    const el = audioRef.current;
    if (!el) return;
    setCurrent(el.currentTime);
    const now = Date.now();
    if (now - lastSaveRef.current >= SAVE_INTERVAL_MS) {
      lastSaveRef.current = now;
      persist(el.currentTime, el.duration || duration, el.ended);
    }
  }, [persist, duration]);

  const handleLoadedMetadata = useCallback(() => {
    const el = audioRef.current;
    if (!el) return;
    setDuration(el.duration || 0);
    // Apply a restored position once we know the media is seekable.
    if (resumeToRef.current != null) {
      try {
        el.currentTime = resumeToRef.current;
      } catch {
        /* seeking may be unavailable until further buffering — non-fatal */
      }
      resumeToRef.current = null;
    }
  }, []);

  const togglePlay = useCallback(() => {
    const el = audioRef.current;
    if (!el) return;
    if (el.paused) void el.play();
    else el.pause();
  }, []);

  const skip = useCallback((delta: number) => {
    const el = audioRef.current;
    if (!el) return;
    const target = Math.max(0, Math.min(el.duration || 0, el.currentTime + delta));
    el.currentTime = target;
    setCurrent(target);
  }, []);

  const seekTo = useCallback(
    (seconds: number, save = false) => {
      const el = audioRef.current;
      if (!el) return;
      el.currentTime = seconds;
      setCurrent(seconds);
      if (save) persist(seconds, el.duration || duration, false);
    },
    [persist, duration],
  );

  const changeSpeed = useCallback((rate: number) => {
    const el = audioRef.current;
    if (el) el.playbackRate = rate;
    setSpeed(rate);
  }, []);

  // Keyboard: space = play/pause, arrows = skip, Esc = close.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      // Don't hijack typing in inputs.
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

  // Save on pause and on unmount so a position isn't lost between throttle ticks.
  const handlePause = useCallback(() => {
    setPlaying(false);
    const el = audioRef.current;
    if (el) persist(el.currentTime, el.duration || duration, el.ended);
  }, [persist, duration]);

  const activeChapterIndex = useMemo(() => {
    if (chapters.length === 0) return -1;
    return chapters.findIndex((c) => current >= c.start && current < c.end);
  }, [chapters, current]);

  const jumpToChapter = useCallback(
    (chapter: AudioChapter) => {
      seekTo(chapter.start, true);
      const el = audioRef.current;
      if (el && el.paused) void el.play();
    },
    [seekTo],
  );

  const percent = duration > 0 ? Math.round((current / duration) * 100) : 0;

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
        {restored && (
          <audio
            ref={audioRef}
            src={src}
            preload="metadata"
            onLoadedMetadata={handleLoadedMetadata}
            onTimeUpdate={handleTimeUpdate}
            onPlay={() => setPlaying(true)}
            onPause={handlePause}
            onEnded={() => {
              setPlaying(false);
              const el = audioRef.current;
              if (el) persist(el.duration || duration, el.duration || duration, true);
            }}
          />
        )}

        <div className="player__seek">
          <span className="player__time">{formatTime(current)}</span>
          <input
            className="player__scrub"
            type="range"
            min={0}
            max={Math.max(0, duration)}
            step={0.1}
            value={Math.min(current, duration || current)}
            aria-label="Seek"
            onChange={(e) => seekTo(Number(e.target.value))}
          />
          <span className="player__time">{formatTime(duration)}</span>
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
