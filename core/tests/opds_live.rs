//! Live-network smoke test for the OPDS connector.
//!
//! Points the connector at a **real public** OPDS feed (Project Gutenberg, a
//! public-domain catalog) and crawls to an acquisition feed, proving the
//! nav→acquisition discovery, pagination, and entry→`Book` mapping against a
//! live server. Marked `#[ignore]` so the default suite stays offline. Run:
//!
//! ```text
//! cargo test -p libro-core --test opds_live -- --ignored --nocapture
//! ```
//!
//! Calibre-Web / Kavita / Komga verification stays pending the user's own
//! instance; the wire format is identical OPDS 1.2, so this exercises the same
//! code paths.

use libro_core::providers::opds::{OpdsConfig, OpdsProvider};
use libro_core::providers::Provider;

#[tokio::test]
#[ignore = "live network"]
async fn crawls_project_gutenberg_opds() {
    let provider = OpdsProvider::new(OpdsConfig {
        feed_url: "https://www.gutenberg.org/ebooks.opds/".to_string(),
        username: None,
        password: None,
    });

    let books = provider.list_library().await.expect("crawl failed");
    println!("[opds] discovered {} books", books.len());
    assert!(!books.is_empty(), "expected at least one book from a public feed");

    for b in books.iter().take(3) {
        println!(
            "[opds book]\n  title: {}\n  authors: {:?}\n  cover: {:?}\n  acquisition: {:?}\n  type: {:?}",
            b.title,
            b.authors,
            b.cover_url,
            b.identifiers.get("opds:acquisition_url"),
            b.media_type,
        );
        assert!(!b.title.is_empty());
        assert!(b.identifiers.contains_key("opds:acquisition_url"));
    }
}
