# Plugin test fixtures

These files exercise the plugin SDK (`src/lib/plugins/`) end-to-end in unit tests. They are
**test fixtures only** — nothing here is imported by the app or bundled into `dist/` (the WASM
provider receives its module bytes at runtime from on-device user config, never from a
build-time import).

## `example-catalog.wasm`

The prebuilt example **WASM guest** for plugin runtime v1. Its provenance is our own: it is the
compiled output of the blueprint's `plugins/examples/wasm-catalog` Rust crate (branch
`origin/jrmoulckers-scaffold-libro-skeleton`, committed as `plugins/example-wasm-catalog.wasm`),
authored by the Libro contributors. It is **not** a third-party binary and contains no DRM
circumvention — it simply calls the host-mediated `host_http_get("{base_url}/api/books")`,
parses a demo server's JSON, and returns a normalized book array.

It implements ABI v1 exactly (`plugin_abi_version`/`alloc`/`list_catalog` exports;
`host_http_get`/`host_http_body`/`host_log` imports), which is why the very same guest that ran
under the blueprint's `wasmi` host runs unchanged under this project's native-`WebAssembly`
host. The runtime tests instantiate it with a fake `HostHttp` (no network).

## `*.plugin.json`

Example manifests in Libro's own (browser) manifest schema — one per kind (`declarative`,
`wasm`). `manifest.test.ts` reads both files and asserts they satisfy `validatePluginManifest`,
so the schema and these examples cannot drift apart silently.

They are deliberately read from disk rather than inlined. Every other manifest in the suite is
an object literal built by a helper, which tests the validator but leaves these committed files
unchecked — which is what they were before that test existed.
