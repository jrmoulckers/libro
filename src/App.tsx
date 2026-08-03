import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AudioChapter, Book, BookMetadata, KindleConfig, PlaybackManifest, PlaybackTrack, PluginInfo, ProviderBooks, SendOutcome } from "./types";
import { EpubReader, type EpubSource } from "./EpubReader";
import { AudioPlayer } from "./AudioPlayer";
import { isTauri } from "./tauri";
import "./App.css";

/** What the reader is currently showing (null = library view). */
interface ReaderState {
  source: EpubSource;
  title: string;
  bookId?: string;
  /** Full catalog book, for opt-in reading-progress sync (omitted for the sample). */
  book?: Book;
}

/** What the audio player is currently playing (null = not playing). */
interface PlayerState {
  tracks: PlaybackTrack[];
  totalDuration: number;
  title: string;
  chapters: AudioChapter[];
  bookId?: string;
  book?: Book;
}

/**
 * The bundled synthetic sample audiobook, used to demo the player in a plain
 * browser (`npm run dev`) with no backend. THREE short public-domain tone
 * segments (`/sample-audiobook-1..3.wav`, ~3s each) are laid out on one
 * book-absolute timeline so auto-advance and cross-boundary seek/chapter-jump
 * are all exercisable. `start_offset_seconds` is the cumulative sum of prior
 * durations, exactly like the Rust `assemble_timeline` helper produces for ABS.
 */
const SAMPLE_TRACK_SECONDS = 3;
const SAMPLE_TRACKS: PlaybackTrack[] = [1, 2, 3].map((n, i) => ({
  index: i,
  url: `/sample-audiobook-${n}.wav`,
  duration_seconds: SAMPLE_TRACK_SECONDS,
  start_offset_seconds: i * SAMPLE_TRACK_SECONDS,
  mime_type: "audio/wav",
}));
const SAMPLE_TOTAL = SAMPLE_TRACKS.length * SAMPLE_TRACK_SECONDS; // 9s

/**
 * Chapter markers for the sample. "Crossing" deliberately spans the 3s track
 * boundary (2s–5s) so jumping to it and playing through it exercises the
 * cross-boundary chapter + auto-advance logic.
 */
const SAMPLE_CHAPTERS: AudioChapter[] = [
  { id: 0, start: 0, end: 2, title: "Prologue" },
  { id: 1, start: 2, end: 5, title: "Crossing (spans a track boundary)" },
  { id: 2, start: 5, end: SAMPLE_TOTAL, title: "Finale" },
];

/** Empty Send-to-Kindle form (587 = the common STARTTLS submission port). */
const EMPTY_KINDLE_CONFIG: KindleConfig = {
  smtp_host: "",
  smtp_port: 587,
  smtp_username: "",
  smtp_password: "",
  from_address: "",
  to_address: "",
};

/** Turn a typed `SendOutcome` into a user-facing status line. */
function describeSendOutcome(outcome: SendOutcome, title: string): string {
  switch (outcome.status) {
    case "sent":
      return `Sent “${title}” to your Kindle. It may take a minute to appear.`;
    case "not_configured":
      return "Send-to-Kindle isn't configured yet — add your SMTP details first.";
    case "too_large":
      return `“${title}” is too large (${(outcome.size / 1048576).toFixed(1)} MB, limit ${(outcome.limit / 1048576).toFixed(0)} MB).`;
    case "not_an_epub":
      return `“${title}” isn't a DRM-free EPUB, so it can't be sent.`;
    case "send_failed":
      return `Couldn't send “${title}”: ${outcome.reason}`;
  }
}

function App() {
  const [providers, setProviders] = useState<ProviderBooks[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reading phase: the currently-open EPUB, if any.
  const [reader, setReader] = useState<ReaderState | null>(null);
  const [openingId, setOpeningId] = useState<string | null>(null);

  // Playback phase: the currently-playing audiobook, if any.
  const [player, setPlayer] = useState<PlayerState | null>(null);

  // Metadata enrichment demo (Open Library / Google Books).
  const [query, setQuery] = useState("");
  const [meta, setMeta] = useState<BookMetadata[]>([]);
  const [metaLoading, setMetaLoading] = useState(false);
  const [metaError, setMetaError] = useState<string | null>(null);

  // Phase 3: installed connector plugins (declarative manifests).
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);

  // Send-to-Kindle: whether it's configured, the settings form, and send status.
  const [kindleConfigured, setKindleConfigured] = useState(false);
  const [showKindleSettings, setShowKindleSettings] = useState(false);
  const [kindleForm, setKindleForm] = useState<KindleConfig>(EMPTY_KINDLE_CONFIG);
  const [kindleSaving, setKindleSaving] = useState(false);
  const [kindleMsg, setKindleMsg] = useState<string | null>(null);
  const [sendingId, setSendingId] = useState<string | null>(null);

  const loadLibrary = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // Fans out over every configured provider in the Rust core and returns
      // each provider's result (books + optional per-provider error) so the UI
      // can degrade gracefully when one connector is offline.
      const result = await invoke<ProviderBooks[]>("list_books_by_provider");
      setProviders(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const searchMetadata = useCallback(async () => {
    if (!query.trim()) return;
    setMetaLoading(true);
    setMetaError(null);
    try {
      const results = await invoke<BookMetadata[]>("search_metadata", { query });
      setMeta(results);
    } catch (e) {
      setMetaError(String(e));
    } finally {
      setMetaLoading(false);
    }
  }, [query]);

  useEffect(() => {
    void loadLibrary();
  }, [loadLibrary]);

  // Load the installed plugins list (Tauri only; the browser demo has no
  // on-device plugins directory).
  useEffect(() => {
    if (!isTauri()) return;
    invoke<PluginInfo[]>("list_plugins")
      .then(setPlugins)
      .catch((e) => console.warn("list_plugins failed", e));
  }, []);

  // Load Send-to-Kindle status + settings (Tauri only). The backend returns the
  // config with the SMTP password blanked, so the secret never reaches the UI.
  const refreshKindle = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const configured = await invoke<boolean>("kindle_configured");
      setKindleConfigured(configured);
      const cfg = await invoke<KindleConfig>("get_kindle_config");
      setKindleForm({ ...EMPTY_KINDLE_CONFIG, ...cfg, smtp_password: "" });
    } catch (e) {
      console.warn("kindle status failed", e);
    }
  }, []);

  useEffect(() => {
    void refreshKindle();
  }, [refreshKindle]);

  const saveKindleConfig = useCallback(async () => {
    setKindleSaving(true);
    setKindleMsg(null);
    try {
      // A blank password means "keep the stored one" on the backend.
      await invoke("save_kindle_config", { configIn: kindleForm });
      setKindleMsg("Send-to-Kindle settings saved.");
      await refreshKindle();
    } catch (e) {
      setKindleMsg(`Couldn't save settings: ${String(e)}`);
    } finally {
      setKindleSaving(false);
    }
  }, [kindleForm, refreshKindle]);

  // Email a local DRM-free EPUB to the user's @kindle.com address. This is a
  // user-initiated action, so the backend returns a typed outcome we surface.
  const sendToKindle = useCallback(async (book: Book) => {
    setSendingId(book.id);
    setKindleMsg(null);
    try {
      const outcome = await invoke<SendOutcome>("send_to_kindle", {
        bookId: book.id,
        title: book.title,
      });
      setKindleMsg(describeSendOutcome(outcome, book.title));
    } catch (e) {
      setKindleMsg(`Couldn't send “${book.title}”: ${String(e)}`);
    } finally {
      setSendingId(null);
    }
  }, []);

  // Open a locally-scanned EPUB: fetch its bytes from the Rust core and hand the
  // ArrayBuffer to the reader. Only usable for Local Files books under Tauri.
  const openBook = useCallback(async (book: Book) => {
    setOpeningId(book.id);
    setError(null);
    try {
      const bytes = await invoke<number[]>("get_book_file", { bookId: book.id });
      const buffer = new Uint8Array(bytes).buffer;
      setReader({
        source: buffer,
        title: book.title,
        bookId: book.id,
        book,
      });
    } catch (e) {
      setError(`Couldn't open “${book.title}”: ${String(e)}`);
    } finally {
      setOpeningId(null);
    }
  }, []);

  // Open the bundled public-domain sample so the reader is demoable in a plain
  // browser (`npm run dev`) without the Tauri backend.
  const openSample = useCallback(() => {
    setReader({ source: "/sample.epub", title: "The Fables of Aesop (sample)" });
  }, []);

  // Open an Audiobookshelf audiobook: resolve a playable stream + chapters from
  // the Rust core, then hand them to the audio player. Only for ABS audiobooks
  // under Tauri.
  const openAudiobook = useCallback(async (book: Book) => {
    setOpeningId(book.id);
    setError(null);
    try {
      const manifest = await invoke<PlaybackManifest>("get_audiobook_stream", {
        bookId: book.id,
      });
      setPlayer({
        tracks: manifest.tracks,
        totalDuration: manifest.total_duration,
        title: book.title,
        chapters: manifest.chapters,
        bookId: book.id,
        book,
      });
    } catch (e) {
      setError(`Couldn't play “${book.title}”: ${String(e)}`);
    } finally {
      setOpeningId(null);
    }
  }, []);

  // Open the bundled public-domain sample audiobook so the player is demoable in
  // a plain browser without the Tauri backend.
  const openSampleAudiobook = useCallback(() => {
    setPlayer({
      tracks: SAMPLE_TRACKS,
      totalDuration: SAMPLE_TOTAL,
      title: "Synthetic Sample Audiobook",
      chapters: SAMPLE_CHAPTERS,
    });
  }, []);

  const canRead = (book: Book) =>
    book.media_type === "Ebook" &&
    book.source_provider_id === "localfiles" &&
    isTauri();

  const canListen = (book: Book) =>
    book.media_type === "Audiobook" &&
    book.source_provider_id === "audiobookshelf" &&
    isTauri();

  // Send-to-Kindle is offered only for the user's own local DRM-free EPUBs, and
  // only once the SMTP/Kindle settings are configured.
  const canSendToKindle = (book: Book) =>
    book.media_type === "Ebook" &&
    book.source_provider_id === "localfiles" &&
    isTauri() &&
    kindleConfigured;

  const books = useMemo<Book[]>(
    () => providers.flatMap((p) => p.books),
    [providers],
  );
  const failed = useMemo(() => providers.filter((p) => p.error), [providers]);

  if (reader) {
    return (
      <EpubReader
        source={reader.source}
        title={reader.title}
        bookId={reader.bookId}
        book={reader.book}
        onClose={() => setReader(null)}
      />
    );
  }

  if (player) {
    return (
      <AudioPlayer
        key={player.bookId ?? player.tracks[0]?.url ?? "sample"}
        tracks={player.tracks}
        totalDuration={player.totalDuration}
        title={player.title}
        chapters={player.chapters}
        bookId={player.bookId}
        book={player.book}
        onClose={() => setPlayer(null)}
      />
    );
  }

  return (
    <main className="app">
      <header className="app__header">
        <h1>Libro</h1>
        <p className="app__tagline">Your library, aggregated. Pure client.</p>
        <div className="app__header-actions">
          <button onClick={() => void loadLibrary()} disabled={loading}>
            {loading ? "Loading…" : "Refresh library"}
          </button>
          <button onClick={openSample}>Open sample book</button>
          <button onClick={openSampleAudiobook}>Open sample audiobook</button>
          {isTauri() && (
            <button onClick={() => setShowKindleSettings((s) => !s)}>
              {showKindleSettings ? "Hide Kindle settings" : "Send-to-Kindle settings"}
            </button>
          )}
        </div>
      </header>

      {kindleMsg && <p className="app__notice">{kindleMsg}</p>}

      {isTauri() && showKindleSettings && (
        <section className="app__kindle">
          <h2>Send-to-Kindle</h2>
          <p className="app__tagline">
            Email your own DRM-free EPUBs to your <code>@kindle.com</code> address
            over your own SMTP account, using Amazon's official personal-documents
            flow. Your <code>from</code> address must be an{" "}
            <em>Approved Personal Document Email</em> in your Amazon account.
          </p>
          <form
            className="app__kindle-form"
            onSubmit={(e) => {
              e.preventDefault();
              void saveKindleConfig();
            }}
          >
            <label>
              SMTP host
              <input
                type="text"
                value={kindleForm.smtp_host}
                placeholder="smtp.gmail.com"
                onChange={(e) => setKindleForm({ ...kindleForm, smtp_host: e.target.value })}
              />
            </label>
            <label>
              SMTP port
              <input
                type="number"
                value={kindleForm.smtp_port}
                onChange={(e) =>
                  setKindleForm({ ...kindleForm, smtp_port: Number(e.target.value) })
                }
              />
            </label>
            <label>
              SMTP username
              <input
                type="text"
                value={kindleForm.smtp_username}
                autoComplete="username"
                onChange={(e) => setKindleForm({ ...kindleForm, smtp_username: e.target.value })}
              />
            </label>
            <label>
              SMTP password
              <input
                type="password"
                value={kindleForm.smtp_password}
                autoComplete="new-password"
                placeholder="•••••••• (unchanged)"
                onChange={(e) => setKindleForm({ ...kindleForm, smtp_password: e.target.value })}
              />
            </label>
            <label>
              From (approved sender)
              <input
                type="email"
                value={kindleForm.from_address}
                placeholder="you@example.com"
                onChange={(e) => setKindleForm({ ...kindleForm, from_address: e.target.value })}
              />
            </label>
            <label>
              To (@kindle.com)
              <input
                type="email"
                value={kindleForm.to_address}
                placeholder="you@kindle.com"
                onChange={(e) => setKindleForm({ ...kindleForm, to_address: e.target.value })}
              />
            </label>
            <button type="submit" disabled={kindleSaving}>
              {kindleSaving ? "Saving…" : "Save settings"}
            </button>
          </form>
        </section>
      )}

      {error && <p className="app__error">Failed to load: {error}</p>}

      <section className="app__metadata">
        <h2>Metadata lookup</h2>
        <p className="app__tagline">
          Search official public catalogs (Open Library, Google Books) — used to
          enrich books with covers, descriptions, and identifiers.
        </p>
        <form
          className="app__metadata-form"
          onSubmit={(e) => {
            e.preventDefault();
            void searchMetadata();
          }}
        >
          <input
            type="text"
            value={query}
            placeholder="Title, author, or ISBN…"
            onChange={(e) => setQuery(e.target.value)}
          />
          <button type="submit" disabled={metaLoading || !query.trim()}>
            {metaLoading ? "Searching…" : "Search"}
          </button>
        </form>

        {metaError && <p className="app__error">Lookup failed: {metaError}</p>}

        {meta.length > 0 && (
          <ul className="app__metadata-results">
            {meta.map((m, i) => (
              <li key={`${m.source}:${m.identifiers.olid ?? m.identifiers.google_volume_id ?? i}`}>
                <strong>{m.title}</strong>
                {m.authors.length > 0 && <> — {m.authors.join(", ")}</>}
                {m.publish_date && <> ({m.publish_date})</>}
                <span className="app__metadata-source"> via {m.source}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {plugins.length > 0 && (
        <section className="app__plugins">
          <h2>Installed plugins</h2>
          <p className="app__tagline">
            Declarative connectors added without recompiling Libro. Each is
            sandboxed to the network domains its manifest declares.
          </p>
          <ul className="app__plugin-list">
            {plugins.map((p) => (
              <li key={p.id}>
                <strong>{p.name}</strong> <span>v{p.version}</span>
                {p.author && <> — {p.author}</>}
                <span className="app__plugin-domains">
                  {" "}
                  reaches: {p.allowed_domains.join(", ")}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {failed.length > 0 && (
        <section className="app__provider-errors">
          {failed.map((p) => (
            <p key={p.provider_id} className="app__error">
              <strong>{p.display_name}</strong>: {p.error}
            </p>
          ))}
        </section>
      )}

      {!loading && !error && books.length === 0 && (
        <p className="app__empty">
          No books yet. Configure a provider to start aggregating your library.
        </p>
      )}

      <section className="grid">
        {books.map((book) => (
          <article key={`${book.source_provider_id}:${book.id}`} className="card">
            <div className="card__cover">
              {book.cover_url ? (
                <img src={book.cover_url} alt={book.title} />
              ) : (
                <span className="card__cover-placeholder">{book.media_type}</span>
              )}
            </div>
            <div className="card__body">
              <h2 className="card__title">{book.title}</h2>
              <p className="card__author">
                {book.authors.length > 0 ? book.authors.join(", ") : "Unknown author"}
              </p>
              <p className="card__source">via {book.source_provider_id}</p>
              {canRead(book) && (
                <button
                  className="card__read"
                  onClick={() => void openBook(book)}
                  disabled={openingId === book.id}
                >
                  {openingId === book.id ? "Opening…" : "Read"}
                </button>
              )}
              {canListen(book) && (
                <button
                  className="card__read"
                  onClick={() => void openAudiobook(book)}
                  disabled={openingId === book.id}
                >
                  {openingId === book.id ? "Loading…" : "Listen"}
                </button>
              )}
              {canSendToKindle(book) && (
                <button
                  className="card__send-kindle"
                  onClick={() => void sendToKindle(book)}
                  disabled={sendingId === book.id}
                >
                  {sendingId === book.id ? "Sending…" : "Send to Kindle"}
                </button>
              )}
            </div>
          </article>
        ))}
      </section>
    </main>
  );
}

export default App;
