import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Book, ProviderBooks } from "./types";
import "./App.css";

function App() {
  const [providers, setProviders] = useState<ProviderBooks[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  useEffect(() => {
    void loadLibrary();
  }, [loadLibrary]);

  const books = useMemo<Book[]>(
    () => providers.flatMap((p) => p.books),
    [providers],
  );
  const failed = useMemo(() => providers.filter((p) => p.error), [providers]);

  return (
    <main className="app">
      <header className="app__header">
        <h1>Libro</h1>
        <p className="app__tagline">Your library, aggregated. Pure client.</p>
        <button onClick={() => void loadLibrary()} disabled={loading}>
          {loading ? "Loading…" : "Refresh library"}
        </button>
      </header>

      {error && <p className="app__error">Failed to load: {error}</p>}

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
            </div>
          </article>
        ))}
      </section>
    </main>
  );
}

export default App;
