//! Catalog enrichment pass — the bridge between library `Provider`s and the
//! metadata layer.
//!
//! After the aggregation fan-out ([`crate::providers::Provider::list_library`])
//! collects a `Vec<Book>`, [`enrich_catalog`] runs each book through a
//! [`MetadataRegistry`] and fills in the fields the source provider left blank
//! (cover, series, description, identifiers) — never overwriting existing data.
//!
//! Design goals baked in here:
//! * **Failure isolation** — a metadata miss or API error never drops or breaks a
//!   catalog book; the book is returned un-enriched.
//! * **Politeness** — lookups run with *bounded* concurrency
//!   ([`futures::stream::StreamExt::buffer_unordered`]); we never fire an
//!   unbounded burst at the public APIs.
//! * **Per-run cache** — lookup keys are de-duplicated across the whole batch so
//!   the same ISBN/query is only ever fetched once per run.
//! * **Skip cheaply** — books that are already complete, or that have neither a
//!   usable identifier nor a usable title+author, are skipped without any call.

use std::collections::HashMap;

use futures::stream::{self, StreamExt};

use crate::models::Book;

use super::{enrich, BookMetadata, MetadataRegistry};

/// Tuning knobs for [`enrich_catalog`].
#[derive(Debug, Clone)]
pub struct EnrichOptions {
    /// When `false`, [`enrich_catalog`] returns the books untouched.
    pub enabled: bool,
    /// Maximum number of metadata lookups in flight at once.
    pub concurrency: usize,
    /// Result cap for the title+author `search` fallback.
    pub search_limit: usize,
}

impl Default for EnrichOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            concurrency: 5,
            search_limit: 5,
        }
    }
}

/// How a given book should be resolved against the metadata providers.
///
/// De-duplicated across the batch so identical lookups collapse to one request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum LookupKey {
    /// Resolve by exact ISBN (preferred — unambiguous).
    Isbn(String),
    /// Fall back to a free-text `title author` search.
    Search(String),
}

/// Enrich a batch of catalog books with missing fields from `registry`.
///
/// Order is preserved and the output always has exactly the same books (same
/// length) as the input — enrichment only augments, never adds or drops entries.
pub async fn enrich_catalog(
    registry: &MetadataRegistry,
    books: Vec<Book>,
    options: &EnrichOptions,
) -> Vec<Book> {
    if !options.enabled {
        return books;
    }

    // Phase 1: derive a lookup key per book (or `None` to skip it).
    let keys: Vec<Option<LookupKey>> = books.iter().map(lookup_key_for).collect();

    // Phase 2: collapse to the unique set so each ISBN/query is fetched once.
    let mut unique: Vec<LookupKey> = Vec::new();
    for key in keys.iter().flatten() {
        if !unique.contains(key) {
            unique.push(key.clone());
        }
    }
    if unique.is_empty() {
        return books;
    }

    // Phase 3: resolve unique keys with bounded concurrency into a per-run cache.
    let concurrency = options.concurrency.max(1);
    let search_limit = options.search_limit;
    let resolved: HashMap<LookupKey, Option<BookMetadata>> = stream::iter(unique)
        .map(|key| async move {
            let meta = resolve_key(registry, &key, search_limit).await;
            (key, meta)
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // Phase 4: apply enrichment in the original order.
    books
        .into_iter()
        .zip(keys)
        .map(|(mut book, key)| {
            if let Some(key) = key {
                if let Some(Some(meta)) = resolved.get(&key) {
                    enrich(&mut book, meta);
                }
            }
            book
        })
        .collect()
}

/// Does this book already have every field enrichment would fill?
fn is_complete(book: &Book) -> bool {
    !book.authors.is_empty()
        && book.cover_url.is_some()
        && book.description.is_some()
        && book.series.is_some()
}

/// Pick the best lookup strategy for a book, or `None` if it can't/shouldn't be
/// enriched (already complete, or lacking any usable key).
fn lookup_key_for(book: &Book) -> Option<LookupKey> {
    if is_complete(book) {
        return None;
    }
    // Prefer an exact ISBN (13, then 10).
    if let Some(isbn) = book
        .identifiers
        .get("isbn13")
        .or_else(|| book.identifiers.get("isbn10"))
    {
        let isbn = isbn.trim();
        if !isbn.is_empty() {
            return Some(LookupKey::Isbn(isbn.to_string()));
        }
    }
    // Otherwise fall back to a title+author search.
    let title = book.title.trim();
    if !title.is_empty() && !book.authors.is_empty() {
        let author = book.authors[0].trim();
        let query = if author.is_empty() {
            title.to_string()
        } else {
            format!("{title} {author}")
        };
        return Some(LookupKey::Search(query));
    }
    None
}

/// Resolve a single [`LookupKey`]. Errors are swallowed (logged) and treated as a
/// miss so one flaky lookup can't sink the batch.
async fn resolve_key(
    registry: &MetadataRegistry,
    key: &LookupKey,
    search_limit: usize,
) -> Option<BookMetadata> {
    match key {
        LookupKey::Isbn(isbn) => match registry.by_isbn(isbn).await {
            Ok(meta) => meta,
            Err(e) => {
                eprintln!("libro: enrichment by_isbn('{isbn}') error: {e}");
                None
            }
        },
        LookupKey::Search(query) => match registry.search(query, search_limit).await {
            Ok(results) => results.into_iter().next(),
            Err(e) => {
                eprintln!("libro: enrichment search('{query}') error: {e}");
                None
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{MetadataError, MetadataProvider, MetadataResult};
    use crate::models::{Book, MediaType};
    use async_trait::async_trait;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// An in-memory [`MetadataProvider`] with call counters — no network.
    struct FakeProvider {
        by_isbn_calls: Arc<AtomicUsize>,
        search_calls: Arc<AtomicUsize>,
        isbn_map: HashMap<String, BookMetadata>,
        search_map: HashMap<String, Vec<BookMetadata>>,
        fail: bool,
    }

    impl FakeProvider {
        fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
            let by_isbn_calls = Arc::new(AtomicUsize::new(0));
            let search_calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    by_isbn_calls: by_isbn_calls.clone(),
                    search_calls: search_calls.clone(),
                    isbn_map: HashMap::new(),
                    search_map: HashMap::new(),
                    fail: false,
                },
                by_isbn_calls,
                search_calls,
            )
        }
    }

    #[async_trait]
    impl MetadataProvider for FakeProvider {
        fn id(&self) -> &str {
            "fake"
        }
        async fn by_isbn(&self, isbn: &str) -> MetadataResult<Option<BookMetadata>> {
            self.by_isbn_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(MetadataError::Api("boom".into()));
            }
            Ok(self.isbn_map.get(isbn).cloned())
        }
        async fn search(&self, query: &str, _limit: usize) -> MetadataResult<Vec<BookMetadata>> {
            self.search_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(MetadataError::Api("boom".into()));
            }
            Ok(self.search_map.get(query).cloned().unwrap_or_default())
        }
    }

    fn meta(title: &str) -> BookMetadata {
        BookMetadata {
            title: title.into(),
            authors: vec!["Joshua Bloch".into()],
            description: Some("desc".into()),
            cover_url: Some("https://cover/L.jpg".into()),
            identifiers: BTreeMap::from([("isbn13".to_string(), "9780134685991".to_string())]),
            source: "fake".into(),
            ..Default::default()
        }
    }

    fn book_missing(id: &str, isbn: Option<&str>) -> Book {
        let mut b = Book::new(id, "Effective Java", MediaType::Ebook, "audiobookshelf");
        b.authors = vec!["Joshua Bloch".into()];
        if let Some(isbn) = isbn {
            b.identifiers.insert("isbn13".into(), isbn.into());
        }
        b
    }

    #[tokio::test]
    async fn fills_only_missing_fields() {
        let (mut fake, isbn_calls, _) = FakeProvider::new();
        fake.isbn_map
            .insert("9780134685991".into(), meta("Effective Java"));
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        let mut book = book_missing("1", Some("9780134685991"));
        book.cover_url = Some("https://existing/cover.jpg".into()); // pre-set: keep
        let out = enrich_catalog(&registry, vec![book], &EnrichOptions::default()).await;

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].description.as_deref(), Some("desc")); // filled
        assert_eq!(out[0].cover_url.as_deref(), Some("https://existing/cover.jpg")); // preserved
        assert_eq!(isbn_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn provider_error_is_tolerated() {
        let (mut fake, _, _) = FakeProvider::new();
        fake.fail = true;
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        let book = book_missing("1", Some("9780134685991"));
        let out = enrich_catalog(&registry, vec![book.clone()], &EnrichOptions::default()).await;

        assert_eq!(out.len(), 1);
        assert_eq!(out[0], book); // returned unchanged, not dropped
    }

    #[tokio::test]
    async fn miss_is_tolerated() {
        let (fake, isbn_calls, _) = FakeProvider::new(); // empty maps -> miss
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        let book = book_missing("1", Some("9780000000000"));
        let out = enrich_catalog(&registry, vec![book.clone()], &EnrichOptions::default()).await;

        assert_eq!(out, vec![book]);
        assert_eq!(isbn_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn per_run_cache_dedupes_identical_lookups() {
        let (mut fake, isbn_calls, _) = FakeProvider::new();
        fake.isbn_map
            .insert("9780134685991".into(), meta("Effective Java"));
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        // Two different books that resolve to the SAME ISBN.
        let a = book_missing("a", Some("9780134685991"));
        let b = book_missing("b", Some("9780134685991"));
        let out = enrich_catalog(&registry, vec![a, b], &EnrichOptions::default()).await;

        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|bk| bk.description.as_deref() == Some("desc")));
        // Only ONE network call despite two books.
        assert_eq!(isbn_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn disabled_flag_skips_the_pass() {
        let (fake, isbn_calls, search_calls) = FakeProvider::new();
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        let book = book_missing("1", Some("9780134685991"));
        let opts = EnrichOptions {
            enabled: false,
            ..Default::default()
        };
        let out = enrich_catalog(&registry, vec![book.clone()], &opts).await;

        assert_eq!(out, vec![book]);
        assert_eq!(isbn_calls.load(Ordering::SeqCst), 0);
        assert_eq!(search_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn already_complete_book_is_skipped() {
        let (fake, isbn_calls, search_calls) = FakeProvider::new();
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        let mut book = book_missing("1", Some("9780134685991"));
        book.cover_url = Some("c".into());
        book.description = Some("d".into());
        book.series = Some("s".into()); // now complete
        let out = enrich_catalog(&registry, vec![book.clone()], &EnrichOptions::default()).await;

        assert_eq!(out, vec![book]);
        assert_eq!(isbn_calls.load(Ordering::SeqCst), 0);
        assert_eq!(search_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn search_fallback_when_no_isbn() {
        let (mut fake, isbn_calls, search_calls) = FakeProvider::new();
        fake.search_map.insert(
            "Effective Java Joshua Bloch".into(),
            vec![meta("Effective Java")],
        );
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        let book = book_missing("1", None); // no ISBN -> search path
        let out = enrich_catalog(&registry, vec![book], &EnrichOptions::default()).await;

        assert_eq!(out[0].description.as_deref(), Some("desc"));
        assert_eq!(isbn_calls.load(Ordering::SeqCst), 0);
        assert_eq!(search_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrency_preserves_all_books_and_order() {
        let (mut fake, _, _) = FakeProvider::new();
        for i in 0..20 {
            fake.isbn_map
                .insert(format!("isbn-{i}"), meta(&format!("Book {i}")));
        }
        let registry = MetadataRegistry::new(vec![Box::new(fake)]);

        let books: Vec<Book> = (0..20)
            .map(|i| book_missing(&format!("id-{i}"), Some(&format!("isbn-{i}"))))
            .collect();
        let ids: Vec<String> = books.iter().map(|b| b.id.clone()).collect();

        let out = enrich_catalog(&registry, books, &EnrichOptions::default()).await;

        assert_eq!(out.len(), 20);
        // Same books, same order, none dropped or duplicated.
        assert_eq!(out.iter().map(|b| b.id.clone()).collect::<Vec<_>>(), ids);
        assert!(out.iter().all(|b| b.description.as_deref() == Some("desc")));
    }
}
