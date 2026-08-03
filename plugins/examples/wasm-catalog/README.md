# Example WASM catalog plugin (guest)

An end-to-end example of Libro's **v2 WASM plugin kind**. It mirrors the
declarative `plugins/example-rest-catalog.json`, but as *code*: its
`list_catalog` export calls the host-mediated `host_http_get("{base_url}/api/books")`,
parses the server's JSON, and returns a normalized `Book` array.

It compiles to `wasm32-unknown-unknown` and is **excluded from the host
workspace** (see the root `Cargo.toml` `exclude`), so `cargo check --workspace`
never tries to build it for the host target.

## Building the committed fixture

The runtime tests run against the **prebuilt, committed**
`plugins/example-wasm-catalog.wasm`, so a wasm toolchain is *not* needed to run
the suite. To rebuild it after changing this crate:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown \
  --manifest-path plugins/examples/wasm-catalog/Cargo.toml
# then copy the artifact next to its manifest:
cp target/wasm32-unknown-unknown/release/libro_wasm_catalog_example.wasm \
  plugins/example-wasm-catalog.wasm
```

(On Windows PowerShell, use `Copy-Item` for the last step. The target-add works
under any host toolchain, including `x86_64-pc-windows-gnu`.)

## ABI

See `core/src/plugins/wasm.rs` for the authoritative ABI v1 contract:

- **Exports:** `memory`, `plugin_abi_version() -> i32`, `alloc(i32) -> i32`,
  `list_catalog(cfg_ptr: i32, cfg_len: i32) -> i64` (packed `(ptr<<32)|len`).
- **Imports (`env`):** `host_http_get(url_ptr, url_len) -> i64`,
  `host_http_body(dst_ptr, dst_len) -> i32`, `host_log(ptr, len)`.
