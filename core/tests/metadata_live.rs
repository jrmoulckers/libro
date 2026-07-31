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

use libro_core::metadata::{GoogleBooksProvider, MetadataProvider, OpenLibraryProvider};

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
