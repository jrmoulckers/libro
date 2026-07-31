//! Live-network smoke tests for the metadata providers.
//!
//! These hit the **real** Open Library and Google Books APIs, so they are marked
//! `#[ignore]` and excluded from the default (offline) suite. Run them manually:
//!
//! ```text
//! cargo test -p libro-core --test metadata_live -- --ignored --nocapture
//! ```
//!
//! They print the normalized [`BookMetadata`] they resolve so a human can eyeball
//! live success. Google Books may return HTTP 429 from shared/cloud IPs without
//! an API key; that is an environmental rate limit, not a code failure.

use libro_core::metadata::{
    enrich_catalog, EnrichOptions, GoogleBooksProvider, MetadataProvider, MetadataRegistry,
    OpenLibraryProvider,
};
use libro_core::models::{Book, MediaType};

const KNOWN_ISBN: &str = "9780134685991"; // Effective Java, 3rd ed.

#[tokio::test]
#[ignore = "live network"]
async fn openlibrary_by_isbn_live() {
    let ol = OpenLibraryProvider::new();
    let meta = ol
        .by_isbn(KNOWN_ISBN)
        .await
        .expect("open library request failed")
        .expect("expected a hit for a well-known ISBN");
    println!("[open library by_isbn] {meta:#?}");
    assert!(!meta.title.is_empty());
    assert_eq!(meta.source, "openlibrary");
}

#[tokio::test]
#[ignore = "live network"]
async fn openlibrary_search_live() {
    let ol = OpenLibraryProvider::new();
    let results = ol
        .search("effective java bloch", 3)
        .await
        .expect("open library search failed");
    println!("[open library search] {} result(s):", results.len());
    for m in &results {
        println!("  - {} ({:?})", m.title, m.authors);
    }
    assert!(!results.is_empty());
}

#[tokio::test]
#[ignore = "live network"]
async fn googlebooks_by_isbn_live() {
    let gb = GoogleBooksProvider::new(None);
    match gb.by_isbn(KNOWN_ISBN).await {
        Ok(Some(meta)) => {
            println!("[google books by_isbn] {meta:#?}");
            assert!(!meta.title.is_empty());
            assert_eq!(meta.source, "googlebooks");
        }
        Ok(None) => println!("[google books by_isbn] no result"),
        // Anonymous quota may be exhausted on shared IPs; log rather than fail.
        Err(e) => println!("[google books by_isbn] error (likely rate limit): {e}"),
    }
}

#[tokio::test]
#[ignore = "live network"]
async fn googlebooks_search_live() {
    let gb = GoogleBooksProvider::new(None);
    match gb.search("effective java bloch", 3).await {
        Ok(results) => {
            println!("[google books search] {} result(s):", results.len());
            for m in &results {
                println!("  - {} ({:?})", m.title, m.authors);
            }
        }
        Err(e) => println!("[google books search] error (likely rate limit): {e}"),
    }
}

/// End-to-end enrichment: a bare catalog `Book` (only title/author/ISBN, no
/// cover or description) is run through the REAL enrichment pass and should come
/// back with a live cover and — via the Open Library Works endpoint — a
/// description.
#[tokio::test]
#[ignore = "live network"]
async fn enrich_catalog_live_before_after() {
    // "A Brief History of Time" — chosen deliberately: its Open Library *edition*
    // record has no description, but the linked *work* does, so this exercises the
    // edition -> work fallback added to OpenLibraryProvider::by_isbn.
    const ENRICH_ISBN: &str = "9780553380163";

    // A book as a source connector (e.g. Audiobookshelf) might hand it to us:
    // title + author + ISBN, but missing cover/description/series.
    let mut book = Book::new(
        "abs-1",
        "A Brief History of Time",
        MediaType::Ebook,
        "audiobookshelf",
    );
    book.authors = vec!["Stephen Hawking".into()];
    book.identifiers.insert("isbn13".into(), ENRICH_ISBN.into());

    println!("[enrich BEFORE] {book:#?}");

    // Open Library only (no Google Books) so this is deterministic without a key.
    let registry = MetadataRegistry::new(vec![Box::new(OpenLibraryProvider::new())]);
    let out = enrich_catalog(&registry, vec![book], &EnrichOptions::default()).await;
    let after = &out[0];

    println!("[enrich AFTER]  {after:#?}");

    assert_eq!(out.len(), 1);
    assert!(
        after.cover_url.is_some(),
        "expected a live cover to be filled"
    );
    assert!(
        after.description.is_some(),
        "expected a live description via the Works endpoint"
    );
}
