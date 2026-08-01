# Libro — Architecture

Libro is a cross-platform, **pure-client** media hub that aggregates a user's
books and audiobooks across many providers into one normalized catalog.

## Guiding principles

### Pure client — no backend
Libro has **no server**. There is no Libro API, no database server, and no
cloud storage owned by the project. Every connector talks **directly** to its
provider's API from the user's device. This keeps the user in control of their
data and credentials and avoids operating shared infrastructure.

Consequences:
- All state (config, credentials, cached catalog, progress) lives on-device.
- Cross-device continuity is achieved by **device-to-device sync**, not a
  central service (see *Configuration & sync* below).
- Any provider without a client-usable API simply can't be a connector (see
  *Deferred / not possible* below).

### The connector (plugin) contract is first-class
The most important abstraction is the **`Provider`** trait
(`core/src/providers/mod.rs`). Every external source is a `Provider`, and
everything the app can do with a backend flows through this trait. The
abstraction is designed for from day one so new connectors are cheap to add.

A `Provider` advertises `ProviderCapabilities` (a bitflags set: `CATALOG`,
`HOLDS`, `REQUEST`, `DOWNLOAD`, `SEND_TO_KINDLE`, `PROGRESS_SYNC`,
`DEEP_LINK_ONLY`) so the UI can enable/disable actions per provider without
probing. `DEEP_LINK_ONLY` marks a walled provider that Libro can only *link out*
to (its official app), never integrate against — see *Legal boundaries*. Methods
are `async` (`async_trait`) because real connectors do network I/O, and the trait
is object-safe so providers can be stored as `Box<dyn Provider>` in a registry.

Adding a connector:
1. Define a config struct for its settings.
2. Implement `Provider`, declaring the right capabilities.
3. Add one arm to the registry in `commands.rs::build_providers`.

### The plugin SDK — connectors without forking core

Phase 3 adds a **declarative plugin SDK** (`core/src/plugins/`) so a user can add
a new library source *without* recompiling Libro or shipping native code. A
plugin is a single JSON **manifest** dropped into the on-device plugins directory
(`<data_dir>/Libro/plugins/`). At startup the loader discovers, parses, and
validates every manifest; a valid one becomes a `PluginProvider` registered in
`build_providers` alongside the native connectors, and is listed via the
`list_plugins` command.

**Mechanism choice — declarative manifest, not WASM (for v1).** Three options
were weighed against Libro's constraints (must run on iOS/Android, must be
sandboxed, must build under this repo's `x86_64-pc-windows-gnu` toolchain):
- **WASM (Extism/wasmtime)** — verified it *does* `cargo check` under
  windows-gnu, but rejected for v1: (1) iOS forbids JIT, and the default
  Extism/wasmtime backend is a Cranelift JIT — a non-starter on a required mobile
  target; (2) it pulls ~300 transitive crates, bloating the already
  export-limited GNU cdylib and the audit surface; (3) a manifest is *inert data*
  (no arbitrary code) so the host mediates every network call — a **tighter**
  sandbox than sandboxing arbitrary Wasm.
- **Subprocess / JSON-RPC** — rejected: mobile sandboxes forbid spawning
  arbitrary child processes.
- **Declarative manifest engine** — chosen: a pure-Rust interpreter over a JSON
  manifest describing a REST/JSON catalog + a field→`Book` mapping. No heavy
  deps, mobile-friendly, and inherently sandboxable.

WASM (for plugins that need real logic beyond declarative mapping) is the
documented next step — it would slot in as an alternative engine behind the same
`Provider` boundary.

**Manifest schema** (`PluginManifest`): `id`, `name`, `version`, `author`,
`plugin_api_version` (must equal `PLUGIN_API_VERSION`, currently `1`), requested
`capabilities`, `permissions.allowed_domains`, a `config_schema` (the user-filled
fields: `base_url`, `api_key`, …, each typed text/secret/url), and a `catalog`
spec: a templated `request` (`{key}` tokens interpolated from config) plus a
`fields` map of dotted JSON paths (e.g. `series.name`) onto the normalized `Book`.

**Validation** rejects malformed or over-broad manifests: wrong api version,
empty id/name, id with whitespace, missing `catalog` capability, empty
`allowed_domains`, any domain that is a wildcard or carries a scheme/path/port,
an empty request URL, or missing id/title field maps. One bad manifest is
skipped (logged), never fatal.

**Sandbox / security boundary.** A plugin gets **no** ambient network or
filesystem. The engine interpolates the config into the request, then
**enforces the domain allowlist on the resolved URL before any request is sent**
— a host not covered by `allowed_domains` (matched exactly or as a subdomain)
returns a typed error, never a panic. Plugins honor the **same legal rules as
native connectors** (see *Legal boundaries*): user-owned services and
official/public APIs only, sandboxed to declared domains, **no** bundled
scrapers/indexers/sources and **no** DRM circumvention. The plugin system must
not become a backdoor for shipping illicit sources.

**Authoring a plugin.** Write a manifest (see `plugins/example-rest-catalog.json`
for a complete, offline-tested example), declare only the domain(s) it needs,
map the response fields onto `Book`, and drop it in the plugins directory. No
build step. The example maps a generic REST catalog
(`GET {base_url}/api/books` → `results[]`) into `Book`s and is exercised
end-to-end from fixture JSON in the test suite (no network).

**TODOs:** a WASM runtime path; plugin signing/verification; a discovery
registry/marketplace; hot-reload; richer per-permission prompts; `POST`/paged
requests.

### Normalized catalog & aggregation
Connectors map their native API responses into a single provider-agnostic domain
model (`core/src/models`): `Book` (id, title, authors, series, cover,
cross-provider `identifiers` like ISBN/ASIN, `media_type`, `source_provider_id`,
`progress`), plus `MediaType` and `Progress`.

Aggregation (`commands.rs::list_all_books`) fans out over every configured,
enabled provider, authenticates it, pulls `list_library()`, and merges the
results into one `Vec<Book>`. Per-provider failures are surfaced (via
`list_books_by_provider`) or logged and skipped (via `list_all_books`) so a
single broken connector can't sink the whole catalog. A later phase will use the
`identifiers` map to **de-duplicate** the same title arriving from multiple
providers.

### Metadata enrichment — a separate abstraction

Not every external catalog is a *library*. Open Library and Google Books are
**bibliographic reference APIs**: they describe books (covers, descriptions,
identifiers, page counts) but they do not list *the user's owned copies* and take
no part in the `list_all_books` fan-out. Forcing them through `Provider` /
`list_library()` would be a category error.

So metadata lives in its own module (`core/src/metadata/`), a sibling of
`core/src/providers/`, with its own trait:

```
trait MetadataProvider {
    async fn by_isbn(&self, isbn) -> Result<Option<BookMetadata>>;
    async fn search(&self, query, limit) -> Result<Vec<BookMetadata>>;
    async fn by_identifier(&self, kind, value) -> Result<Option<BookMetadata>>;
}
```

- `BookMetadata` is a richer, read-only descriptor (title, subtitle, authors,
  description, cover, series, `identifiers` map, publish date, page count,
  publisher, language, `source`). A *miss* is `Ok(None)` / empty `Vec`, **not** an
  error; only network/transport/API failures are `MetadataError`.
- `enrich(book: &mut Book, meta: &BookMetadata)` fills **only missing** fields on
  a normalized `Book` (authors if empty; cover/description/series if absent;
  identifiers via `or_insert`) and never overwrites existing values. This is the
  seam catalog connectors (ABS / LazyLibrarian) reuse to backfill sparse records.
- `MetadataRegistry` consults enabled providers in priority order
  (Open Library first, then Google Books), returning the first hit; per-provider
  errors are logged and treated as misses so one slow source can't block a lookup.

Implemented providers (both **official public APIs**, no scraping):

- **Open Library** (no auth): search `GET /search.json`, ISBN via the Books API
  `GET /api/books?bibkeys=ISBN:{isbn}&format=json&jscmd=data`, covers via
  `https://covers.openlibrary.org/b/id/{cover_i}-L.jpg`. Sends a descriptive
  `User-Agent` per their etiquette. *(TODO: `jscmd=data` returns no description;
  a follow-up can hit the Works endpoint for that.)*
- **Google Books** (optional `api_key` raises limits):
  `GET /books/v1/volumes?q=isbn:{isbn}` / `?q={query}`, mapping `volumeInfo`.
  Cover URLs are upgraded `http`→`https`.

Exposed to the frontend via the `search_metadata` and `lookup_metadata_by_isbn`
Tauri commands. Configuration is a single optional Google Books `api_key` under
`AppConfig.metadata`; Open Library needs none.

**Catalog enrichment pass.** After the `list_all_books` fan-out merges the
per-provider `Vec<Book>`, it runs through `metadata::enrich_catalog`, which
backfills each book's missing cover/description/series/identifiers from the
registry. It is deliberately defensive:

- **Failure isolation** — a miss or API error never drops or mutates a book; it
  is returned un-enriched.
- **Bounded concurrency** — lookups run via `futures` `buffer_unordered` (cap 5),
  never an unbounded burst at the public APIs.
- **Per-run dedupe cache** — books are reduced to a unique set of lookup keys
  (ISBN first, else a `title author` search) so an identifier/query is only
  fetched once per run; results are applied back in original order.
- **Cheap skip** — books already complete, or with no usable identifier *and* no
  usable title+author, are skipped without a call.
- Toggle with `metadata.enrich_catalog` (default `true`).

Because Open Library's `jscmd=data` payload carries no description, the
`OpenLibraryProvider` follows the edition doc (`/books/{olid}.json`) and, if
needed, the linked work (`/works/{id}.json`) to fill `description` best-effort.

### Configuration & sync
`core/src/config` defines `AppConfig` (a list of `ProviderConfig` entries:
type + opaque per-provider settings blob) and the save/load **boundary**.

Target design (not yet implemented — the module is a typed stub):
- Config is **encrypted at rest**; the data-encryption key is held in the OS
  keychain (Keychain / Windows Credential Manager / libsecret / mobile secure
  enclave).
- Cross-device recovery/sync uses a **user-controlled, Signal-style encrypted
  backup blob** rather than a central service.
- No real cryptography is implemented yet; `load_config`/`save_config` are the
  seams where it will live.

### Progress sync (two-way)
Libro's on-device stores (`ReadingStore`, `ListeningStore`) are always the
**source of truth**. Around them sit *optional, opt-in* sync engines in two
directions — **outward** (push local progress up) and **inbound** (pull remote
progress down + reconcile) — sharing one design so they stay consistent.

#### Outward (push)
Two engines mirror local progress back **out** to a user-owned/official service:

- **Reading → Hardcover** (`core/src/sync.rs`) — when reading progress is saved,
  best-effort set the Hardcover shelf status (0→>0 ⇒ *currently-reading*,
  finished ⇒ *read*) behind the `ProgressTracker` trait (implemented by
  `HardcoverProvider`; tests inject a fake). It **resolves** the local `Book` to a
  Hardcover book id (ISBN, else title+author) and caches that mapping.
- **Listening → Audiobookshelf** (`core/src/listening_sync.rs`) — when listening
  progress is saved, best-effort `PATCH /api/me/progress/{libraryItemId}` with
  `currentTime` / `duration` / `progress` / `isFinished`, behind the
  `ListeningTracker` trait (implemented by `AudiobookshelfProvider`; tests inject
  a fake). No resolve step is needed — the audiobook `Book.id` **is** the ABS
  `libraryItemId` (the same id `get_audiobook_stream` opens a play session with).
  The pure `map_media_progress_body` helper builds the request body separately
  from the HTTP call so it is unit-tested without a network.

Both engines share the same guarantees:
- **Opt-in, default off** — `hardcover.sync_reading_progress` /
  `audiobookshelf.sync_listening_progress` (both `#[serde(default)]` false).
  Writing to a user's account is never a silent side effect. No-op when the flag
  is off or the provider isn't configured.
- **Failure isolation** — the local save runs first and is the only thing that can
  fail the command; every network/resolve/sync error is logged and swallowed
  (`SyncOutcome` / `ListeningSyncOutcome`: `Disabled` / `NotConfigured` /
  `NoChange` / `Updated` / `Finished` / `Failed`). The reader/player never breaks.
- **Throttle** — per-item last-synced state (interior mutability; the lock is
  never held across an `.await`) means the API is only hit on a real transition or
  a meaningful delta (listening: a position change ≥ 15 s, plus pause / chapter
  change / finish), never on every tick/page turn.

#### Inbound (pull-down + reconcile)
The counterpart, `core/src/progress_sync.rs`, pulls each remote's current
progress **down** on library load and reconciles it against the local store, so a
book advanced on another device resumes at the right place here.

- **Inbound read, pure-mapped** — each provider exposes a `ProgressSource`
  (`fetch_remote_progress(&Book) -> Option<RemoteProgress>`), with the JSON→record
  mapping split from the HTTP call and unit-tested: `map_abs_progress_record` (ABS
  `me.mediaProgress`: `currentTime`/`duration`/`isFinished`/`lastUpdate`, ms→s) and
  `map_status_to_remote_progress` (Hardcover shelf status → coarse fraction:
  *Read* ⇒ finished 1.0, *CurrentlyReading* ⇒ in-progress). Hardcover exposes no
  fine-grained fraction or per-row timestamp here, so its `RemoteProgress.updated_at`
  is `None`.
- **Reconcile policy** — the pure `reconcile(local, remote) -> Reconciliation`
  (`NoData` / `AlreadyInSync` / `LocalWins` / `RemoteWins(Progress)`) decides the
  winner, and **never errors**:
  1. **`finished` is sticky** — once finished anywhere, it stays finished.
  2. **Newest-wins by `updated_at`** (last-write-wins) when *both* sides carry a
     timestamp and they differ by more than a small recency tie window.
  3. **Furthest-position-wins** (max fraction) when timestamps are missing or
     unreliable — the current fallback for both lanes, since the local `Progress`
     model carries no timestamp yet (newest-wins is implemented + tested via
     `reconcile_with` for a future timestamped store).
  4. **Tie/threshold no-thrash** — deltas within `PROGRESS_TIE_EPSILON` ⇒
     `AlreadyInSync`, no write.
- **Lanes never cross** — ABS-sourced audiobooks reconcile against Audiobookshelf
  (seconds / `ListeningStore`); other ebooks reconcile against Hardcover (CFI /
  `ReadingStore`). A Hardcover-sourced book is skipped (`lane_for`).
- **Apply on load, opt-in + bounded + isolated** — the `reconcile_progress`
  command (gated per lane by `pull_progress`, `#[serde(default)]` false) fetches
  remotes with **bounded concurrency** (`buffer_unordered`, cap 6) then applies
  winners **sequentially** (the file stores share a temp path, so serialized
  writes avoid a race). Only `RemoteWins` writes the local store; every error is
  swallowed into a `ReconcileReport` tally and the library load never fails.
- **No feedback loop** — after an audio pull-down the command seeds the outward
  listening throttle (`ListeningSyncState::note_synced_position`), so a reconciled
  position isn't immediately echoed back up to ABS on the next save.

TODOs: real-time/webhook sync (instead of pull-on-load); a manual per-book
conflict-resolution UI (let the user choose when auto last-write-wins isn't
wanted); richer Hardcover fraction + a per-row `updated_at` if the API exposes
page-level progress (which would activate the newest-wins branch for reading); and
session-based ABS progress via the `/api/session` close endpoint if richer than
the `me/progress` PATCH.

## Component overview

The Rust side is a two-crate Cargo workspace:

- **`libro-core`** (`core/`) — all pure-client business logic: the domain model,
  the `Provider` contract, every connector, and the config boundary. It has **no
  Tauri/GUI dependency**, so its mapping/aggregation logic is unit-tested with a
  plain static test binary (`cargo test -p libro-core`) — no WebView runtime
  needed.
- **`libro`** (`src-tauri/`) — the thin Tauri shell. It depends on `libro-core`
  and exposes it to the frontend through the `commands.rs` surface (`run()` plus
  the `#[tauri::command]` functions).

```
┌────────────────────────────┐        invoke("list_all_books")
│  React + TS frontend (src/) │  ───────────────────────────────►
│  App.tsx renders Vec<Book>  │                                   │
└────────────────────────────┘                                   ▼
                        ┌───────────────────────────────────────────────┐
                        │  libro  (src-tauri) — Tauri shell             │
                        │    commands.rs  list_all_books()              │
                        │        │  build_providers(AppConfig)          │
                        └────────┼──────────────────────────────────────┘
                                 ▼
                        ┌───────────────────────────────────────────────┐
                        │  libro-core  (core) — pure client, no GUI     │
                        │    providers::Provider (trait)                │
                        │     ├─ AudiobookshelfProvider (real REST)      │
                        │     ├─ HardcoverProvider (real GraphQL)        │
                        │     ├─ LazyLibrarianProvider (real REST)      │
                        │     ├─ LocalFilesProvider (EPUB on disk)       │
                        │     ├─ OpdsProvider (real OPDS 1.2 Atom)       │
                        │     ├─ LibbyProvider (deep-link-only)          │
                        │     └─ PluginProvider (declarative manifests)  │
                        │          │ list_library()                     │
                        │          ▼                                    │
                        │    models::Book (normalized)                  │
                        │    config:: load/save (encrypted*)            │
                        └───────────────────────────────────────────────┘
                                     │ direct HTTPS (user's own servers / official APIs)
                                     ▼
                        each provider's own API (no Libro server)
```
`*` encryption is a planned boundary, not yet implemented.

## Roadmap (phase order)

1. **Library aggregation** — normalize and merge each provider's library into one
   catalog. *(Audiobookshelf is a real REST connector; live end-to-end verification
   against a running server is pending.)*
2. **Request / acquisition** — request or acquire titles not yet owned (holds,
   downloads).
3. **Plugin / connector system** — harden the `Provider` abstraction, dynamic
   registration, per-connector config UI. *(Shipped v1: a **declarative plugin
   SDK** (`core/src/plugins/`) lets a user add a new connector by dropping a JSON
   manifest into their on-device plugins directory — no recompiling Libro, no
   native code. Each manifest declares an id/name/version, a
   `plugin_api_version`, requested `ProviderCapabilities`, a user-filled config
   schema, sandboxed `allowed_domains`, and a `catalog` spec (a templated REST
   request + a field→`Book` mapping). The `PluginProvider` engine interpolates
   the user's config into the request, enforces the domain allowlist before any
   network call, fetches the JSON, and maps items onto normalized `Book`s. Plugins
   register into `build_providers` alongside native connectors and are exposed via
   the `list_plugins` command. A working example ships at
   `plugins/example-rest-catalog.json`. See "The plugin SDK" below. TODOs: a WASM
   runtime for plugins needing real logic, plugin signing/verification, a discovery
   registry/marketplace, hot-reload, and richer permission prompts.)*
4. **Audiobook playback** — in-app player with progress sync. *(Started: a
   source-agnostic in-app audio player (`src/AudioPlayer.tsx`) plays the user's
   own DRM-free audiobooks via a webview HTML5 `<audio>` element — play/pause,
   scrub, skip ±30s, 0.5×–3× speed, chapter list + jump-to-chapter, time/percent,
   and keyboard control. It takes a stream URL from the Rust
   `get_audiobook_stream` command (which opens an Audiobookshelf `/api/items/{id}/play`
   session and resolves an absolute, token-in-query stream URL + chapters — an
   HTML media element can't send an `Authorization` header) or, for a
   backend-free browser demo, a bundled synthetic public-domain sample at
   `public/sample-audiobook.wav`. Listening position (seconds + percent) is
   persisted per-book — throttled, plus on pause/chapter change — via
   `save_listening_progress` / `get_listening_progress`, backed by an on-device
   `ListeningStore` (`core/src/config/listening.rs`), kept parallel to the reading
   store so one item can hold independent reading and listening positions. No DRM
   handling. Pending: live ABS streaming needs a running server; multi-track
   (multi-file) gapless playback uses only the first track for now. **Native TODOs**
   (separate platform work): background playback, lockscreen / now-playing
   controls, Android Auto / Apple CarPlay, Chromecast, sleep timer, equalizer.
   Outward listening-progress **sync-back to Audiobookshelf** is now wired
   (opt-in `PATCH /api/me/progress/{id}`; see *Progress sync (two-way)*); live
   verification is pending a running ABS server.)*
5. **Reading** — EPUB reading experience. *(Started: an in-app EPUB reader
   (`src/EpubReader.tsx`, built on `react-reader`/epub.js) renders the user's own
   DRM-free local EPUBs. It takes its content either as bytes from the Rust
   `get_book_file` command (Local Files connector) or, for a backend-free browser
   demo, from a bundled public-domain sample at `public/sample.epub`. Reading
   position (EPUB CFI + percent) is persisted per-book via
   `save_reading_progress` / `get_reading_progress`, backed by an on-device
   `ReadingStore` (`core/src/config/reading.rs`). No DRM handling. Reading
   progress **syncs back to Hardcover** (opt-in; see *Progress sync
   (two-way)*). TODOs: highlights/annotations, full-text search, and dark mode.)*

## Legal boundaries

Libro must stay **legally clean**. The whole design follows one rule:

> Libro only ever connects to **(a) the user's own self-hosted services and
> files**, and **(b) official, public, documented APIs** — using credentials the
> user supplies.

Concrete boundaries, enforced in code by how each connector is modeled:

- **No scraping and no private/reverse-engineered endpoints.** Libro must not
  call undocumented or internal APIs (e.g. OverDrive/Libby's internal "Thunder"
  API, or scraping StoryGraph's web pages). Walled providers are modeled as a
  **deep link out** to their official app, never as an integration
  (`DEEP_LINK_ONLY`).
- **No DRM handling or circumvention** of any kind.
- **No bundled indexers or content sources.** Connectors that can request or
  download (e.g. LazyLibrarian) talk **only to the user's own self-hosted
  instance**; Libro ships no indexers, trackers, or download sources.
- **User-owned services are fully in scope.** Audiobookshelf and LazyLibrarian
  are things the user runs themselves — connecting to them with the user's own
  token/API key is legitimate.

How specific walled gardens are handled:

- **Libby / OverDrive** — holds and loans live behind an app with no public
  third-party API. Modeled as `DEEP_LINK_ONLY`: Libro stores a library/user
  identifier and builds a link into the official Libby app. It calls **no**
  OverDrive endpoint.
- **Kindle library / Audible** — Amazon exposes no API to read a user's library.
  Deep-link-out or manual import only; never scraped.
- **Send-to-Kindle** — *is* supported, because it uses the official documented
  path: emailing a supported file to the user's `@kindle.com` address
  (`SEND_TO_KINDLE`).
- **Goodreads** — its API was retired to new developers, so it is **not** a
  connector. **Hardcover** (official public GraphQL API) is the reading-tracking
  path instead.
- **StoryGraph** — has no official public API. Libro will only ever ingest the
  user's **own exported CSV** (a manual import of data the user already owns) —
  never live scraping.

## Provider landscape — what's possible

Realistic connector targets, by tier:

*User-owned, self-hosted (official APIs, fully in scope):*
- **Audiobookshelf** — documented REST API; the first **real** connector
  (`CATALOG` + `PROGRESS_SYNC`).
- **LazyLibrarian** — the user's own instance via its REST API
  (`CATALOG`/`REQUEST`/`DOWNLOAD`). *(Context: Readarr was retired June 2025;
  LazyLibrarian and its forks are the living self-hosted path. Libro bundles no
  indexers — it only drives the user's instance.)* **Real connector**
  (`getVersion`, `getAllBooks`, `findBook`, `addBook`, `queueBook`, `searchBook`);
  live verification pending a running instance.
- **Local Files** — the user's own DRM-free EPUB files on disk (`CATALOG`). Scans
  configured folders, parses each EPUB's OPF (via `zip` + `roxmltree`) into a
  normalized `Book` (title, authors, language, publisher, description, ISBN,
  Calibre series, cover). Embedded covers are served to the frontend through the
  `get_local_cover` command behind a `localcover://{book_id}` reference; books
  with no cover/description are left sparse so the metadata-enrichment pass fills
  them. **No DRM handling** — DRM-protected files are read for open metadata only
  or skipped, never decrypted. Fully local; live-verified end-to-end via the
  enrichment synergy test.
- **OPDS** — a generic connector for the [OPDS](https://specs.opds.io/) standard
  (`CATALOG` + `DOWNLOAD`), so one connector reaches **Calibre-Web**, Calibre's
  content server, **Kavita**, **Komga**, and public catalogs (Standard Ebooks,
  Project Gutenberg). Targets **OPDS 1.2 (Atom/XML)**; parses feeds with
  `roxmltree`, distinguishes navigation from acquisition feeds, and does a
  **bounded** crawl (depth + total-page cap) following `rel="next"` pagination to
  discover books. Maps each acquisition entry → normalized `Book` (title, authors,
  description, ISBN, series best-effort, absolute cover URL, and the primary
  acquisition link/type for `DOWNLOAD`). Supports HTTP Basic auth (Calibre-Web's
  default) and unauthenticated public feeds. Live-verified against Project
  Gutenberg; Calibre-Web-specific verification pending the user's own instance.
  *TODO:* OPDS 2.0 (JSON), download-to-disk + UI, richer series extraction.

*Official public APIs (user-supplied key):*
- **Hardcover** — official public GraphQL API for reading status, ratings, and
  shelves (`PROGRESS_SYNC`; not a catalog/holds source). **Real connector**
  (`me`, `user_books`, `search`, `insert_user_book`, `update_user_book`,
  `insert_user_book_read`); live verification pending a user-supplied API key.
- **Open Library** — official public API for bibliographic metadata + covers
  (no auth). **Real** `MetadataProvider` (see *Metadata enrichment*); live-verified.
- **Google Books** — official public API for metadata (optional key). **Real**
  `MetadataProvider`; live calls rate-limited without a key in some environments.
- **Send-to-Kindle** — official email-to-`@kindle.com` path (`SEND_TO_KINDLE`).

*Walled gardens (deep-link-out / manual-import only — see Legal boundaries):*
- **Libby / OverDrive** — `DEEP_LINK_ONLY` link into the official app. **Placeholder.**
- **Kindle library / Audible** — deep-link-out or manual import only.
- **StoryGraph** — user CSV import only.

### Deferred / not currently possible
- **Libby / OverDrive, Kindle, Audible** — no public third-party API; can only be
  deep-linked or manually imported (never scraped).
- **Goodreads API** — retired to new developers; **Hardcover** replaces it as the
  tracking path.

## Deliberately deferred implementation

These are intentionally left as TODOs with clear seams:
- Live end-to-end verification of the Audiobookshelf, Hardcover, and LazyLibrarian
  connectors against a running server / real key (each is code-complete and
  unit-tested against captured API response shapes; no live instances are
  available yet). Hardcover's write mutations additionally carry TODOs for a few
  input type names pending its beta schema.
- Real config encryption + OS keychain + Signal-style backup blob (`config`).
- Audio playback and EPUB reading.
- De-duplication of items across providers via `identifiers`.
