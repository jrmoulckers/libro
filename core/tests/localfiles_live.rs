//! Live-network end-to-end proof for the Local Files connector + enrichment pass.
//!
//! Generates a real EPUB on disk that carries an ISBN but NO cover and NO
//! description, scans it via [`LocalFilesProvider`], then runs the resulting
//! catalog through the **real** Open Library enrichment pass and prints the
//! BEFORE/AFTER [`Book`] so a human can confirm the cover + description were
//! filled from the network. Marked `#[ignore]` so the default suite stays
//! offline. Run manually:
//!
//! ```text
//! cargo test -p libro-core --test localfiles_live -- --ignored --nocapture
//! ```

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use libro_core::metadata::{enrich_catalog, EnrichOptions, MetadataRegistry, OpenLibraryProvider};
use libro_core::providers::localfiles::{LocalFilesConfig, LocalFilesProvider};
use libro_core::providers::Provider;
use zip::write::SimpleFileOptions;

const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

// ISBN of "A Brief History of Time" — Open Library has both an edition record
// (cover) AND a work description for it, so the Works-endpoint description
// fallback can be demonstrated end-to-end. (The spec's example 9780134685991 has
// no OL description, only a cover.)
const ISBN: &str = "9780553380163";

fn write_isbn_only_epub(dir: &Path) -> PathBuf {
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>A Brief History of Time</dc:title>
    <dc:creator>Stephen Hawking</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier opf:scheme="ISBN">{ISBN}</dc:identifier>
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="content"/></spine>
</package>"#
    );

    let path = dir.join("brief-history.epub");
    let file = fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();
    zip.start_file("META-INF/container.xml", stored).unwrap();
    zip.write_all(CONTAINER_XML.as_bytes()).unwrap();
    zip.start_file("OEBPS/content.opf", stored).unwrap();
    zip.write_all(opf.as_bytes()).unwrap();
    zip.finish().unwrap();
    path
}

#[tokio::test]
#[ignore = "live network"]
async fn local_epub_gets_enriched_by_open_library() {
    let tmp = tempfile::tempdir().unwrap();
    write_isbn_only_epub(tmp.path());

    let provider = LocalFilesProvider::new(LocalFilesConfig {
        library_paths: vec![tmp.path().to_path_buf()],
    });
    let books = provider.list_library().await.expect("scan failed");
    assert_eq!(books.len(), 1);

    let before = books[0].clone();
    println!("[BEFORE enrichment] {before:#?}");
    assert!(before.cover_url.is_none(), "EPUB had no embedded cover");
    assert!(before.description.is_none(), "EPUB had no description");
    assert_eq!(before.identifiers.get("isbn13").map(String::as_str), Some(ISBN));

    let registry = MetadataRegistry::new(vec![Box::new(OpenLibraryProvider::new())]);
    let enriched = enrich_catalog(&registry, books, &EnrichOptions::default()).await;
    let after = &enriched[0];
    println!("[AFTER enrichment]  {after:#?}");

    assert!(
        after.cover_url.is_some(),
        "expected Open Library to supply a cover URL"
    );
    assert!(
        after.description.is_some(),
        "expected Open Library Works endpoint to supply a description"
    );
}
