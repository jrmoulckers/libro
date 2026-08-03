//! The **WASM plugin runtime** — the second plugin kind (v2), behind the same
//! plugin boundary as the declarative-manifest [`engine`](super::engine).
//!
//! A declarative manifest maps a REST/JSON catalog with dotted field paths; a
//! WASM plugin instead ships a sandboxed `.wasm` module that implements the
//! catalog contract **in code**, for connectors whose logic exceeds declarative
//! mapping. Both kinds present as the same [`Provider`], are capability-scoped,
//! and are sandboxed to the manifest's `allowed_domains`.
//!
//! # Runtime choice: `wasmi` (pure-Rust interpreter, no JIT)
//!
//! The module runs on [`wasmi`], a pure-Rust Wasm **interpreter**. This is the
//! deciding property: **iOS forbids JIT**, so wasmtime's default Cranelift JIT
//! (the reason v1 rejected WASM) is a non-starter — whereas an interpreter
//! executes bytecode with no code generation and runs identically on
//! iOS/Android/desktop. `wasmi` is also light and pure Rust, so it builds under
//! this project's `x86_64-pc-windows-gnu` toolchain (no cranelift/LLVM/MSVC).
//! (wasmtime's experimental Pulley interpreter is not yet a stable story, and
//! wasm3 is C — needing a C toolchain and an `unsafe` FFI surface.)
//!
//! # Sandbox (host-enforced — the guest is untrusted)
//!
//! The module gets **no ambient capabilities**: no direct network, filesystem,
//! clock, or env. Its only outside reach is the host functions imported into it:
//!
//! * [`host_http_get`](HostImports) — the guest passes a URL; the **host**
//!   resolves it, checks the host against the manifest's `allowed_domains`
//!   (reusing the exact same [`check_domain_allowed`](super::engine) subdomain
//!   rules as the declarative engine), performs the fetch host-side, and stashes
//!   the body. A denied domain is **unreachable** from inside the module.
//! * `host_http_body` — copies the last fetched body into guest memory.
//! * `host_log` — debug logging.
//!
//! Resource bounds stop a malicious/buggy module from harming the host:
//! **fuel metering** ([`Config::consume_fuel`]) bounds execution so an infinite
//! loop terminates with a typed [`WasmError::Fuel`] instead of hanging; a
//! **memory cap** ([`StoreLimits`]) bounds guest linear memory; and response +
//! output sizes are bounded too.
//!
//! # ABI v1 (versioned, documented)
//!
//! Strings/JSON cross the boundary as `(ptr, len)` into guest linear memory,
//! using a guest-exported bump allocator. A packed `i64` return carries a
//! pointer/length pair as `(ptr << 32) | len`.
//!
//! **Guest exports:**
//! * `memory` — the linear memory.
//! * `plugin_abi_version() -> i32` — must equal [`WASM_ABI_VERSION`]; the host
//!   rejects a mismatch.
//! * `alloc(len: i32) -> i32` — allocate `len` bytes, return a pointer. The host
//!   uses it to hand the config JSON to the guest, and the guest uses it for the
//!   HTTP-body buffer.
//! * `list_catalog(cfg_ptr: i32, cfg_len: i32) -> i64` — receives the user config
//!   JSON (already written at `cfg_ptr`), returns a packed `(ptr, len)` to a
//!   UTF-8 JSON **array of books** in guest memory. `0` signals an error.
//!
//! **Host imports (module `"env"`):**
//! * `host_http_get(url_ptr: i32, url_len: i32) -> i64` — fetch after the
//!   allowlist check; returns the body length (`>= 0`) or `-1` on denial/error.
//! * `host_http_body(dst_ptr: i32, dst_len: i32) -> i32` — copy the stashed body
//!   into guest memory; returns bytes copied.
//! * `host_log(ptr: i32, len: i32)` — log a UTF-8 string.
//!
//! The `(ptr, len)` scheme is deliberately the simplest robust choice; a richer
//! ABI (pagination, POST, downloads) is a documented TODO in `ARCHITECTURE.md`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use wasmi::{Caller, Config, Engine, Extern, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode};

use crate::models::{Book, MediaType};
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

use super::engine::check_domain_allowed;
use super::PluginManifest;

/// The WASM guest ABI version this host implements. A module's
/// `plugin_abi_version` export must equal this or the host rejects it.
pub const WASM_ABI_VERSION: u32 = 1;

/// Max guest linear memory (bytes). Bounds a memory-bomb module.
const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
/// Fuel budget for a `list_catalog` call. Bounds execution so a runaway/infinite
/// loop terminates with [`WasmError::Fuel`] instead of hanging the host.
const LIST_CATALOG_FUEL: u64 = 500_000_000;
/// Fuel for the tiny `plugin_abi_version` call.
const ABI_FUEL: u64 = 1_000_000;
/// Max HTTP response body the host will hand back to the guest.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Max serialized guest output (the returned book-array JSON).
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// Max config JSON handed into the guest.
const MAX_CONFIG_BYTES: usize = 64 * 1024;
/// Max URL length the guest may pass to `host_http_get`.
const MAX_URL_BYTES: usize = 8 * 1024;
/// Max size of a `.wasm` module we will load from disk.
pub const MAX_WASM_BYTES: usize = 16 * 1024 * 1024;

/// Errors from loading or running a WASM plugin. Every variant is typed — the
/// runtime never panics on guest misbehavior.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("wasm module failed to compile/instantiate: {0}")]
    Instantiate(String),
    #[error("wasm plugin ABI mismatch: module reports {found}, host implements {expected}")]
    AbiMismatch { found: u32, expected: u32 },
    #[error("wasm module is missing the required export '{0}'")]
    MissingExport(&'static str),
    #[error("wasm plugin exceeded its fuel budget (possible infinite loop)")]
    Fuel,
    #[error("wasm guest trapped: {0}")]
    Trap(String),
    #[error("wasm guest memory access error: {0}")]
    Memory(String),
    #[error("wasm guest produced invalid output: {0}")]
    Output(String),
    #[error("host http error: {0}")]
    Http(String),
}

impl From<WasmError> for ProviderError {
    fn from(e: WasmError) -> Self {
        match e {
            WasmError::Http(m) => ProviderError::Network(m),
            WasmError::AbiMismatch { .. } | WasmError::MissingExport(_) | WasmError::Instantiate(_) => {
                ProviderError::Config(e.to_string())
            }
            other => ProviderError::Other(other.to_string()),
        }
    }
}

/// The **host HTTP seam**: performs a GET *after* the allowlist check has passed.
/// A real implementation does the network I/O; tests use a fake that replays a
/// captured fixture, so the suite never touches the network.
pub trait HostHttp: Send + Sync {
    /// Fetch `url` (already allowlist-approved) and return the response bytes.
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// The default, real HTTP host using a **blocking** reqwest client (the wasmi
/// interpreter runs synchronously, so a blocking fetch keeps the host bridge
/// simple; the call is bounded by [`MAX_RESPONSE_BYTES`]).
pub struct ReqwestBlockingHttp;

impl HostHttp for ReqwestBlockingHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = reqwest::blocking::Client::new()
            .get(url)
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("unexpected status {}", resp.status().as_u16()));
        }
        let bytes = resp.bytes().map_err(|e| e.to_string())?;
        Ok(bytes.to_vec())
    }
}

/// Per-instantiation host state carried in the wasmi [`Store`].
struct HostState {
    allowed_domains: Vec<String>,
    http: Box<dyn HostHttp>,
    /// The last fetched body, awaiting a `host_http_body` copy into guest memory.
    last_body: Vec<u8>,
    /// A recorded host-side denial/error, surfaced after the guest call returns.
    http_error: Option<String>,
    /// Captured guest `host_log` lines (for debugging/tests).
    logs: Vec<String>,
    limits: StoreLimits,
}

/// Marker for the host import surface (documentation anchor for the ABI).
pub struct HostImports;

/// Read `len` bytes at `ptr` out of the guest memory found on `caller`.
fn read_guest_bytes(
    caller: &Caller<'_, HostState>,
    mem: &wasmi::Memory,
    ptr: i32,
    len: i32,
    max: usize,
) -> Option<Vec<u8>> {
    if ptr < 0 || len < 0 || len as usize > max {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    mem.read(caller, ptr as usize, &mut buf).ok()?;
    Some(buf)
}

fn caller_memory(caller: &Caller<'_, HostState>) -> Option<wasmi::Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(m)) => Some(m),
        _ => None,
    }
}

/// Run a WASM plugin's `list_catalog` against `config`, enforcing the sandbox,
/// fuel, and memory limits, and mapping the returned JSON to normalized
/// [`Book`]s. **Synchronous** (the interpreter has no async); pure w.r.t. the
/// network except through the injected [`HostHttp`] seam.
pub fn run_catalog(
    wasm_bytes: &[u8],
    provider_id: &str,
    media_type: MediaType,
    config: &Value,
    allowed_domains: Vec<String>,
    http: Box<dyn HostHttp>,
) -> Result<Vec<Book>, WasmError> {
    // Engine with fuel metering enabled.
    let mut cfg = Config::default();
    cfg.consume_fuel(true);
    let engine = Engine::new(&cfg);

    let module = Module::new(&engine, wasm_bytes)
        .map_err(|e| WasmError::Instantiate(e.to_string()))?;

    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_MEMORY_BYTES)
        .build();
    let state = HostState {
        allowed_domains,
        http,
        last_body: Vec::new(),
        http_error: None,
        logs: Vec::new(),
        limits,
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limits);

    let mut linker: Linker<HostState> = Linker::new(&engine);
    define_host_imports(&mut linker)?;

    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| WasmError::Instantiate(e.to_string()))?;

    // ABI version handshake.
    let abi = instance
        .get_typed_func::<(), i32>(&store, "plugin_abi_version")
        .map_err(|_| WasmError::MissingExport("plugin_abi_version"))?;
    store.set_fuel(ABI_FUEL).ok();
    let found = abi.call(&mut store, ()).map_err(classify_call_error)? as u32;
    if found != WASM_ABI_VERSION {
        return Err(WasmError::AbiMismatch {
            found,
            expected: WASM_ABI_VERSION,
        });
    }

    // Hand the config JSON to the guest: alloc + write.
    let alloc = instance
        .get_typed_func::<i32, i32>(&store, "alloc")
        .map_err(|_| WasmError::MissingExport("alloc"))?;
    let list = instance
        .get_typed_func::<(i32, i32), i64>(&store, "list_catalog")
        .map_err(|_| WasmError::MissingExport("list_catalog"))?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or(WasmError::MissingExport("memory"))?;

    let cfg_bytes = serde_json::to_vec(config).map_err(|e| WasmError::Output(e.to_string()))?;
    if cfg_bytes.len() > MAX_CONFIG_BYTES {
        return Err(WasmError::Output("config JSON too large".into()));
    }

    // One fuel budget covers alloc + list_catalog (guest-controlled work).
    store
        .set_fuel(LIST_CATALOG_FUEL)
        .map_err(|e| WasmError::Instantiate(e.to_string()))?;

    let cfg_ptr = alloc
        .call(&mut store, cfg_bytes.len() as i32)
        .map_err(classify_call_error)?;
    if cfg_ptr < 0 {
        return Err(WasmError::Memory("guest alloc returned a negative pointer".into()));
    }
    memory
        .write(&mut store, cfg_ptr as usize, &cfg_bytes)
        .map_err(|e| WasmError::Memory(e.to_string()))?;

    let packed = list
        .call(&mut store, (cfg_ptr, cfg_bytes.len() as i32))
        .map_err(classify_call_error)?;

    // A host-side denial/error (e.g. a blocked domain) trumps whatever the guest
    // returned — the denied fetch was unreachable, so surface it clearly.
    if let Some(err) = store.data().http_error.clone() {
        return Err(WasmError::Http(err));
    }

    if packed == 0 {
        return Err(WasmError::Output("guest signalled an error (null result)".into()));
    }
    let ptr = ((packed >> 32) & 0xffff_ffff) as usize;
    let len = (packed & 0xffff_ffff) as usize;
    if len > MAX_OUTPUT_BYTES {
        return Err(WasmError::Output("guest output exceeds the size limit".into()));
    }
    let mut out = vec![0u8; len];
    memory
        .read(&store, ptr, &mut out)
        .map_err(|e| WasmError::Memory(e.to_string()))?;

    map_books_json(provider_id, media_type, &out)
}

/// Wire the host import functions into the linker under module `"env"`.
fn define_host_imports(linker: &mut Linker<HostState>) -> Result<(), WasmError> {
    let wrap = |linker: &mut Linker<HostState>| -> Result<(), wasmi::errors::LinkerError> {
        // host_http_get(url_ptr, url_len) -> i64  (body length, or -1 on error)
        linker.func_wrap(
            "env",
            "host_http_get",
            |mut caller: Caller<'_, HostState>, url_ptr: i32, url_len: i32| -> i64 {
                let Some(mem) = caller_memory(&caller) else { return -1 };
                let Some(url_bytes) = read_guest_bytes(&caller, &mem, url_ptr, url_len, MAX_URL_BYTES)
                else {
                    return -1;
                };
                let Ok(url) = String::from_utf8(url_bytes) else { return -1 };

                // SANDBOX: reuse the declarative engine's exact allowlist check.
                let allowed = caller.data().allowed_domains.clone();
                if check_domain_allowed(&url, &allowed).is_err() {
                    caller.data_mut().http_error = Some(format!(
                        "network permission denied: '{url}' is not in allowed_domains"
                    ));
                    return -1;
                }

                let result = caller.data().http.get(&url);
                match result {
                    Ok(bytes) => {
                        if bytes.len() > MAX_RESPONSE_BYTES {
                            caller.data_mut().http_error =
                                Some("http response exceeds the size limit".into());
                            return -1;
                        }
                        let n = bytes.len() as i64;
                        caller.data_mut().last_body = bytes;
                        n
                    }
                    Err(e) => {
                        caller.data_mut().http_error = Some(e);
                        -1
                    }
                }
            },
        )?;

        // host_http_body(dst_ptr, dst_len) -> i32  (bytes copied)
        linker.func_wrap(
            "env",
            "host_http_body",
            |mut caller: Caller<'_, HostState>, dst_ptr: i32, dst_len: i32| -> i32 {
                if dst_ptr < 0 || dst_len < 0 {
                    return -1;
                }
                let body = std::mem::take(&mut caller.data_mut().last_body);
                let n = body.len().min(dst_len as usize);
                let Some(mem) = caller_memory(&caller) else { return -1 };
                if mem.write(&mut caller, dst_ptr as usize, &body[..n]).is_err() {
                    return -1;
                }
                n as i32
            },
        )?;

        // host_log(ptr, len)
        linker.func_wrap(
            "env",
            "host_log",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
                let Some(mem) = caller_memory(&caller) else { return };
                if let Some(bytes) = read_guest_bytes(&caller, &mem, ptr, len, MAX_URL_BYTES) {
                    if let Ok(s) = String::from_utf8(bytes) {
                        caller.data_mut().logs.push(s);
                    }
                }
            },
        )?;
        Ok(())
    };
    wrap(linker).map_err(|e| WasmError::Instantiate(e.to_string()))
}

/// Turn a wasmi call error into a typed [`WasmError`], distinguishing a fuel
/// exhaustion (bounded infinite loop) from an ordinary guest trap.
fn classify_call_error(err: wasmi::Error) -> WasmError {
    match err.as_trap_code() {
        Some(TrapCode::OutOfFuel) => WasmError::Fuel,
        Some(code) => WasmError::Trap(code.to_string()),
        None => WasmError::Trap(err.to_string()),
    }
}

/// A book as produced by a WASM guest — the normalized field set, minus the
/// `source_provider_id` (the host stamps that) and `media_type` (host-defaulted
/// from the manifest, guest may override).
#[derive(Debug, Deserialize)]
struct WasmBook {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    series: Option<String>,
    #[serde(default)]
    cover_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    identifiers: std::collections::HashMap<String, String>,
    #[serde(default)]
    media_type: Option<MediaType>,
}

/// Parse the guest's JSON book array into normalized [`Book`]s. Pure + typed:
/// malformed JSON or an id/title-less record is an [`WasmError::Output`] /
/// skipped rather than a panic — mirroring the declarative engine's robustness.
pub fn map_books_json(
    provider_id: &str,
    default_media_type: MediaType,
    json_bytes: &[u8],
) -> Result<Vec<Book>, WasmError> {
    let raw: Vec<WasmBook> = serde_json::from_slice(json_bytes)
        .map_err(|e| WasmError::Output(format!("guest output is not a JSON book array: {e}")))?;

    let mut books = Vec::with_capacity(raw.len());
    for wb in raw {
        if wb.id.trim().is_empty() || wb.title.trim().is_empty() {
            continue; // skip id/title-less records, never fatal
        }
        let media = wb.media_type.unwrap_or(default_media_type);
        let mut book = Book::new(wb.id, wb.title, media, provider_id);
        book.authors = wb
            .authors
            .into_iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        book.series = wb.series.filter(|s| !s.is_empty());
        book.cover_url = wb.cover_url.filter(|s| !s.is_empty());
        book.description = wb.description.filter(|s| !s.is_empty());
        for (k, v) in wb.identifiers {
            if !v.is_empty() {
                book.identifiers.insert(k, v);
            }
        }
        books.push(book);
    }
    Ok(books)
}

/// A connector backed by a sandboxed WASM module (plugin kind v2).
///
/// Implements the same [`Provider`] trait as the native connectors and the
/// declarative [`PluginProvider`](super::engine::PluginProvider), so it slots
/// into the provider registry identically and is capability-scoped by its
/// manifest.
pub struct WasmPluginProvider {
    manifest: PluginManifest,
    config: Value,
    wasm_bytes: std::sync::Arc<Vec<u8>>,
    media_type: MediaType,
}

impl WasmPluginProvider {
    /// Create a WASM provider from a validated manifest, its `.wasm` bytes, and
    /// the user's stored settings.
    pub fn new(manifest: PluginManifest, wasm_bytes: Vec<u8>, settings: Value) -> Self {
        let media_type = manifest
            .wasm
            .as_ref()
            .map(|w| w.media_type)
            .unwrap_or(MediaType::Ebook);
        Self {
            manifest,
            config: settings,
            wasm_bytes: std::sync::Arc::new(wasm_bytes),
            media_type,
        }
    }
}

#[async_trait]
impl Provider for WasmPluginProvider {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    fn display_name(&self) -> &str {
        &self.manifest.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.manifest.capability_bits()
    }

    async fn authenticate(&mut self, config: &Value) -> ProviderResult<()> {
        if !config.is_null() {
            self.config = config.clone();
        }
        Ok(())
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        // The wasmi interpreter is synchronous and fuel-bounded. Run it directly
        // with the real blocking HTTP host; the sandbox + fuel + memory caps keep
        // an untrusted module from harming the host.
        let books = run_catalog(
            &self.wasm_bytes,
            &self.manifest.id,
            self.media_type,
            &self.config,
            self.manifest.permissions.allowed_domains.clone(),
            Box::new(ReqwestBlockingHttp),
        )?;
        Ok(books)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Repo-relative path to the prebuilt example `.wasm` fixture (committed so
    /// the suite needs no wasm toolchain — see `plugins/examples/wasm-catalog/`).
    fn fixture_wasm() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("plugins")
            .join("example-wasm-catalog.wasm");
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("read example wasm fixture {}: {e}", path.display()))
    }

    /// A fake HTTP host that replays captured fixture bytes and records the URLs
    /// it was asked to fetch (via a shared handle) — no network.
    struct FakeHttp {
        body: Vec<u8>,
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl FakeHttp {
        fn new(body: &str) -> Self {
            Self {
                body: body.as_bytes().to_vec(),
                seen: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
        fn seen_handle(&self) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
            self.seen.clone()
        }
    }
    impl HostHttp for FakeHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.seen.lock().unwrap().push(url.to_string());
            Ok(self.body.clone())
        }
    }

    // The server returns a NON-normalized shape; the guest maps it to Books.
    fn server_json() -> &'static str {
        r#"{ "items": [
            { "guid": "a1", "name": "Wasm and Order", "writer": "G. Guest", "isbn13": "9780000000009" },
            { "guid": "a2", "name": "The Interpreter", "writer": "P. Pure" }
        ] }"#
    }

    fn config() -> Value {
        serde_json::json!({ "base_url": "https://api.wasm-books.test", "api_key": "K" })
    }

    #[test]
    fn runs_example_wasm_and_maps_books_over_fake_http() {
        let http = FakeHttp::new(server_json());
        let http_seen_handle = http.seen_handle();
        let books = run_catalog(
            &fixture_wasm(),
            "example-wasm-catalog",
            MediaType::Ebook,
            &config(),
            vec!["api.wasm-books.test".into()],
            Box::new(http),
        )
        .expect("guest list_catalog runs and returns books");

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].id, "a1");
        assert_eq!(books[0].title, "Wasm and Order");
        assert_eq!(books[0].authors, vec!["G. Guest"]);
        assert_eq!(books[0].source_provider_id, "example-wasm-catalog");
        assert_eq!(
            books[0].identifiers.get("isbn").map(String::as_str),
            Some("9780000000009")
        );
        // The guest built the URL from base_url and the host performed the fetch.
        let http_seen = http_seen_handle.lock().unwrap().clone();
        assert_eq!(
            http_seen,
            vec!["https://api.wasm-books.test/api/books".to_string()]
        );
    }

    #[test]
    fn denied_domain_is_unreachable_from_the_guest() {
        // The config points base_url at a host NOT on the allowlist. The guest's
        // host_http_get is denied host-side; the run surfaces a typed Http error
        // and NO books leak through.
        let http = FakeHttp::new(server_json());
        let err = run_catalog(
            &fixture_wasm(),
            "example-wasm-catalog",
            MediaType::Ebook,
            &serde_json::json!({ "base_url": "https://evil.example.com", "api_key": "K" }),
            vec!["api.wasm-books.test".into()],
            Box::new(http),
        )
        .unwrap_err();
        match err {
            WasmError::Http(m) => assert!(m.contains("permission denied"), "got: {m}"),
            other => panic!("expected a denied-domain Http error, got {other:?}"),
        }
    }

    // ---- edge-case modules compiled from WAT (no wasm toolchain needed) -------

    fn wat_to_wasm(wat: &str) -> Vec<u8> {
        wat::parse_str(wat).expect("valid WAT")
    }

    #[test]
    fn infinite_loop_hits_the_fuel_limit_and_terminates() {
        // list_catalog loops forever — must terminate via fuel, not hang.
        let wasm = wat_to_wasm(
            r#"(module
                (memory (export "memory") 1)
                (func (export "plugin_abi_version") (result i32) (i32.const 1))
                (func (export "alloc") (param i32) (result i32) (i32.const 0))
                (func (export "list_catalog") (param i32 i32) (result i64)
                    (loop $l (br $l))
                    (i64.const 0)))"#,
        );
        let err = run_catalog(
            &wasm,
            "loop",
            MediaType::Ebook,
            &config(),
            vec!["api.wasm-books.test".into()],
            Box::new(FakeHttp::new("{}")),
        )
        .unwrap_err();
        assert!(matches!(err, WasmError::Fuel), "expected Fuel, got {err:?}");
    }

    #[test]
    fn abi_version_mismatch_is_rejected() {
        let wasm = wat_to_wasm(
            r#"(module
                (memory (export "memory") 1)
                (func (export "plugin_abi_version") (result i32) (i32.const 99))
                (func (export "alloc") (param i32) (result i32) (i32.const 0))
                (func (export "list_catalog") (param i32 i32) (result i64) (i64.const 0)))"#,
        );
        let err = run_catalog(
            &wasm,
            "bad-abi",
            MediaType::Ebook,
            &config(),
            vec!["api.wasm-books.test".into()],
            Box::new(FakeHttp::new("{}")),
        )
        .unwrap_err();
        assert!(matches!(err, WasmError::AbiMismatch { found: 99, expected: 1 }));
    }

    #[test]
    fn malformed_guest_output_is_a_typed_error_not_a_panic() {
        // Returns a packed (ptr=0, len=5) pointing at 5 bytes of non-JSON.
        let wasm = wat_to_wasm(
            r#"(module
                (memory (export "memory") 1)
                (data (i32.const 0) "hello")
                (func (export "plugin_abi_version") (result i32) (i32.const 1))
                (func (export "alloc") (param i32) (result i32) (i32.const 16))
                (func (export "list_catalog") (param i32 i32) (result i64)
                    (i64.const 5)))"#,
        );
        let err = run_catalog(
            &wasm,
            "bad-output",
            MediaType::Ebook,
            &config(),
            vec!["api.wasm-books.test".into()],
            Box::new(FakeHttp::new("{}")),
        )
        .unwrap_err();
        assert!(matches!(err, WasmError::Output(_)), "got {err:?}");
    }

    #[test]
    fn missing_export_is_rejected() {
        let wasm = wat_to_wasm(
            r#"(module
                (memory (export "memory") 1)
                (func (export "plugin_abi_version") (result i32) (i32.const 1)))"#,
        );
        let err = run_catalog(
            &wasm,
            "no-alloc",
            MediaType::Ebook,
            &config(),
            vec!["api.wasm-books.test".into()],
            Box::new(FakeHttp::new("{}")),
        )
        .unwrap_err();
        assert!(matches!(err, WasmError::MissingExport("alloc")), "got {err:?}");
    }

    #[test]
    fn map_books_json_skips_idless_and_parses_identifiers() {
        let json = br#"[
            { "id": "x", "title": "T", "authors": ["A"], "identifiers": { "asin": "B000" } },
            { "id": "", "title": "no id" },
            { "title": "no id field" }
        ]"#;
        let books = map_books_json("p", MediaType::Audiobook, json).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].media_type, MediaType::Audiobook);
        assert_eq!(books[0].identifiers.get("asin").map(String::as_str), Some("B000"));
    }
}
