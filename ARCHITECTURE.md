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
                        │     ├─ LazyLibrarianProvider (stub, REST)      │
                        │     └─ LibbyProvider (deep-link-only)          │
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
   registration, per-connector config UI.
4. **Audiobook playback** — in-app player with progress sync.
5. **Reading** — EPUB reading experience.

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
  indexers — it only drives the user's instance.)* **Stub.**

*Official public APIs (user-supplied key):*
- **Hardcover** — official public GraphQL API for reading status, ratings, and
  shelves (`PROGRESS_SYNC`; not a catalog/holds source). **Real connector**
  (`me`, `user_books`, `search`, `insert_user_book`, `update_user_book`,
  `insert_user_book_read`); live verification pending a user-supplied API key.
- **Open Library** — public API for metadata/covers (planned).
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
- LazyLibrarian REST calls (a typed stub with the right config shape and
  capabilities). *(Hardcover's GraphQL client is now a real connector; a few
  write-mutation input type names are marked TODO pending the beta schema.)*
- Live end-to-end verification of the Audiobookshelf connector against a running
  server (the mapping is code-complete and unit-tested against captured API
  response shapes; no live server is available yet).
- Real config encryption + OS keychain + Signal-style backup blob (`config`).
- Audio playback and EPUB reading.
- De-duplication of items across providers via `identifiers`.
