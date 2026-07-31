import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ReactReader } from "react-reader";
import type { Rendition } from "epubjs";
import type { Progress } from "./types";
import { tryInvoke } from "./tauri";
import "./EpubReader.css";

/**
 * A source the reader can render. Either raw EPUB bytes (the Tauri
 * `get_book_file` path, for the user's own local files) or a URL string (the
 * bundled public-domain `public/sample.epub` browser-demo path). epub.js accepts
 * both, so a single component serves both content sources.
 */
export type EpubSource = ArrayBuffer | string;

export interface EpubReaderProps {
  /** EPUB bytes (local files) or a URL (bundled sample). */
  source: EpubSource;
  /** Book title, shown in the header. */
  title?: string;
  /**
   * Stable book id. When set (and running under Tauri), the reader restores and
   * persists reading progress via the `get_reading_progress` /
   * `save_reading_progress` commands. Omit for the anonymous sample demo.
   */
  bookId?: string;
  /** Close the reader and return to the library. */
  onClose?: () => void;
}

const MIN_FONT = 80;
const MAX_FONT = 200;
const FONT_STEP = 10;

/**
 * In-app EPUB reader (reading phase v1).
 *
 * Wraps `react-reader` (epub.js) to provide paginated rendering, table-of-contents
 * navigation, next/prev (buttons + arrow keys), a font-size control, and a live
 * percentage indicator. Reading position is persisted per-book so the user
 * resumes where they left off.
 *
 * Deliberately NOT in v1 (tracked as TODOs): highlights/annotations, full-text
 * search, themes/dark-mode, and syncing progress to reading trackers
 * (Hardcover / Audiobookshelf).
 */
export function EpubReader({ source, title, bookId, onClose }: EpubReaderProps) {
  const [location, setLocation] = useState<string | number>(0);
  const [percent, setPercent] = useState<number | null>(null);
  const [fontSize, setFontSize] = useState(100);
  const [restored, setRestored] = useState(bookId ? false : true);

  const renditionRef = useRef<Rendition | null>(null);
  const locationsReady = useRef(false);

  // epub.js accepts an ArrayBuffer or a URL string; react-reader's `url` prop is
  // typed as string, so widen it for the bytes path.
  const url = source as string;

  // Restore any saved reading position before we start rendering, so the reader
  // opens where the user left off.
  useEffect(() => {
    let cancelled = false;
    if (!bookId) return;
    void (async () => {
      const progress = await tryInvoke<Progress | null>("get_reading_progress", {
        bookId,
      });
      if (cancelled) return;
      if (progress?.locator) setLocation(progress.locator);
      setRestored(true);
    })();
    return () => {
      cancelled = true;
    };
  }, [bookId]);

  const persist = useCallback(
    (cfi: string, fraction: number) => {
      if (!bookId) return;
      const progress: Progress = {
        fraction,
        locator: cfi,
        position_seconds: null,
        finished: fraction >= 0.999,
      };
      void tryInvoke("save_reading_progress", { bookId, progress });
    },
    [bookId],
  );

  const handleLocationChanged = useCallback(
    (loc: string) => {
      setLocation(loc);
      const rendition = renditionRef.current;
      let fraction: number | null = null;
      if (rendition && locationsReady.current) {
        try {
          fraction = rendition.book.locations.percentageFromCfi(loc);
        } catch {
          fraction = null;
        }
      }
      if (fraction != null && !Number.isNaN(fraction)) {
        setPercent(fraction);
        persist(loc, fraction);
      }
    },
    [persist],
  );

  const getRendition = useCallback((rendition: Rendition) => {
    renditionRef.current = rendition;
    rendition.themes.fontSize(`${fontSize}%`);
    // Generate a coarse locations index so we can report a reading percentage.
    // Best-effort and async; large books just report null until it resolves.
    void rendition.book.ready
      .then(() => rendition.book.locations.generate(1000))
      .then(() => {
        locationsReady.current = true;
      })
      .catch(() => {
        /* percentage stays unavailable — non-fatal */
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Apply font-size changes to the live rendition.
  useEffect(() => {
    renditionRef.current?.themes.fontSize(`${fontSize}%`);
  }, [fontSize]);

  // Keyboard navigation. The EPUB renders in an iframe, so react-reader's own
  // key handling is unreliable; drive prev/next from a document-level listener.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowLeft") renditionRef.current?.prev();
      else if (e.key === "ArrowRight") renditionRef.current?.next();
      else if (e.key === "Escape") onClose?.();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const percentLabel = useMemo(
    () => (percent == null ? "—" : `${Math.round(percent * 100)}%`),
    [percent],
  );

  return (
    <div className="reader">
      <header className="reader__bar">
        <button className="reader__close" onClick={() => onClose?.()}>
          ← Library
        </button>
        <span className="reader__title" title={title}>
          {title ?? "Reading"}
        </span>
        <div className="reader__controls">
          <button
            aria-label="Decrease font size"
            onClick={() => setFontSize((f) => Math.max(MIN_FONT, f - FONT_STEP))}
            disabled={fontSize <= MIN_FONT}
          >
            A−
          </button>
          <span className="reader__font">{fontSize}%</span>
          <button
            aria-label="Increase font size"
            onClick={() => setFontSize((f) => Math.min(MAX_FONT, f + FONT_STEP))}
            disabled={fontSize >= MAX_FONT}
          >
            A+
          </button>
          <span className="reader__percent">{percentLabel}</span>
        </div>
      </header>

      <div className="reader__view">
        {restored && (
          <ReactReader
            url={url}
            title={title}
            location={location}
            locationChanged={handleLocationChanged}
            getRendition={getRendition}
            epubOptions={{ flow: "paginated" }}
          />
        )}
      </div>

      <footer className="reader__foot">
        <button onClick={() => renditionRef.current?.prev()}>‹ Prev</button>
        <button onClick={() => renditionRef.current?.next()}>Next ›</button>
        <span className="reader__hint">Use ← / → keys to turn pages</span>
      </footer>
    </div>
  );
}

export default EpubReader;
