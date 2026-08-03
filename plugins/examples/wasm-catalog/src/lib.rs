//! Example **WASM catalog plugin** (the guest) for Libro's plugin runtime v2.
//!
//! This mirrors the declarative `example-rest-catalog.json`, but as *code*: it
//! calls the host-mediated `host_http_get("{base_url}/api/books")`, parses the
//! server's (non-normalized) JSON, and returns a normalized book array — the
//! same field set the declarative engine maps to. It exists to prove and
//! document the ABI end-to-end; the runtime tests run against the prebuilt
//! `.wasm` produced from this crate.
//!
//! # ABI v1 (see `core/src/plugins/wasm.rs`)
//!
//! * Exports: `memory`, `plugin_abi_version() -> i32`, `alloc(i32) -> i32`,
//!   `list_catalog(cfg_ptr: i32, cfg_len: i32) -> i64` (packed `(ptr<<32)|len`).
//! * Imports (`env`): `host_http_get(url_ptr, url_len) -> i64` (body len, or -1),
//!   `host_http_body(dst_ptr, dst_len) -> i32`, `host_log(ptr, len)`.
//!
//! Strings/JSON cross as `(ptr, len)` into this module's linear memory; the host
//! writes the config into memory we `alloc`, and reads our packed result.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The host ABI version this guest speaks. Must match the host's `WASM_ABI_VERSION`.
const ABI_VERSION: i32 = 1;

#[link(wasm_import_module = "env")]
extern "C" {
    /// Host-mediated GET. The host allowlist-checks the URL, fetches it, stashes
    /// the body, and returns its length (`>= 0`) or `-1` on denial/error.
    fn host_http_get(url_ptr: i32, url_len: i32) -> i64;
    /// Copy the stashed body into guest memory; returns bytes copied.
    fn host_http_body(dst_ptr: i32, dst_len: i32) -> i32;
    /// Log a UTF-8 debug string.
    fn host_log(ptr: i32, len: i32);
}

fn log(msg: &str) {
    unsafe { host_log(msg.as_ptr() as i32, msg.len() as i32) };
}

/// The host checks this before calling anything else.
#[no_mangle]
pub extern "C" fn plugin_abi_version() -> i32 {
    ABI_VERSION
}

/// Allocate `len` bytes in guest memory and return a pointer. The host uses this
/// to hand us the config JSON. We intentionally leak (a plugin instance is
/// short-lived — instantiate, run once, drop the whole store).
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }
    let mut buf: Vec<u8> = Vec::with_capacity(len as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as i32
}

/// The user's config, handed in as JSON.
#[derive(Deserialize)]
struct Config {
    base_url: String,
    #[serde(default)]
    #[allow(dead_code)]
    api_key: Option<String>,
}

/// The server's native (non-normalized) response shape.
#[derive(Deserialize)]
struct ServerResponse {
    #[serde(default)]
    items: Vec<ServerItem>,
}

#[derive(Deserialize)]
struct ServerItem {
    guid: String,
    name: String,
    #[serde(default)]
    writer: Option<String>,
    #[serde(default)]
    isbn13: Option<String>,
}

/// A normalized book, matching the host's expected field set.
#[derive(Serialize)]
struct OutBook {
    id: String,
    title: String,
    authors: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    identifiers: BTreeMap<String, String>,
}

/// The catalog entrypoint: fetch, map, return a packed `(ptr, len)` to a JSON
/// book array in guest memory. `0` signals an error.
#[no_mangle]
pub extern "C" fn list_catalog(cfg_ptr: i32, cfg_len: i32) -> i64 {
    let cfg_bytes = unsafe { std::slice::from_raw_parts(cfg_ptr as *const u8, cfg_len as usize) };
    let cfg: Config = match serde_json::from_slice(cfg_bytes) {
        Ok(c) => c,
        Err(_) => {
            log("bad config JSON");
            return 0;
        }
    };

    let url = format!("{}/api/books", cfg.base_url.trim_end_matches('/'));
    let n = unsafe { host_http_get(url.as_ptr() as i32, url.len() as i32) };
    if n < 0 {
        log("host_http_get failed/denied");
        return 0;
    }

    let mut body = vec![0u8; n as usize];
    let copied = unsafe { host_http_body(body.as_mut_ptr() as i32, body.len() as i32) };
    if copied < 0 {
        return 0;
    }
    body.truncate(copied as usize);

    let resp: ServerResponse = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            log("bad server JSON");
            return 0;
        }
    };

    let books: Vec<OutBook> = resp
        .items
        .into_iter()
        .map(|it| {
            let mut identifiers = BTreeMap::new();
            if let Some(isbn) = it.isbn13.filter(|s| !s.is_empty()) {
                identifiers.insert("isbn".to_string(), isbn);
            }
            OutBook {
                id: it.guid,
                title: it.name,
                authors: it.writer.into_iter().filter(|s| !s.is_empty()).collect(),
                identifiers,
            }
        })
        .collect();

    log(&format!("mapped {} books", books.len()));

    let out = match serde_json::to_vec(&books) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let ptr = out.as_ptr() as u32;
    let len = out.len() as u32;
    std::mem::forget(out);
    ((ptr as i64) << 32) | (len as i64)
}
