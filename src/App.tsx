import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Book } from "./types";
import "./App.css";

function App() {
  const [books, setBooks] = useState<Book[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadLibrary = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // Fans out over every configured provider in the Rust core and returns
      // the merged, normalized catalog.
      const result = await invoke<Book[]>("list_all_books");
      setBooks(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadLibrary();
  }, [loadLibrary]);

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
