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
(`src-tauri/src/providers/mod.rs`). Every external source is a `Provider`, and
everything the app can do with a backend flows through this trait. Even though
only a stub connector ships today, the abstraction is designed for from day one
so new connectors are cheap to add.

A `Provider` advertises `ProviderCapabilities` (a bitflags set: `CATALOG`,
`HOLDS`, `REQUEST`, `DOWNLOAD`, `SEND_TO_KINDLE`, `PROGRESS_SYNC`) so the UI can
enable/disable actions per provider without probing. Methods are `async`
(`async_trait`) because real connectors do network I/O, and the trait is
object-safe so providers can be stored as `Box<dyn Provider>` in a registry.

Adding a connector:
1. Define a config struct for its settings.
2. Implement `Provider`, declaring the right capabilities.
3. Add one arm to the registry in `commands.rs::build_providers`.

### Normalized catalog & aggregation
Connectors map their native API responses into a single provider-agnostic domain
model (`src-tauri/src/models`): `Book` (id, title, authors, series, cover,
cross-provider `identifiers` like ISBN/ASIN, `media_type`, `source_provider_id`,
`progress`), plus `MediaType` and `Progress`.

Aggregation (`commands.rs::list_all_books`) fans out over every configured,
enabled provider, authenticates it, pulls `list_library()`, and merges the
results into one `Vec<Book>`. Per-provider failures are logged and skipped so a
single broken connector can't sink the whole catalog. A later phase will use the
`identifiers` map to **de-duplicate** the same title arriving from multiple
providers.

### Configuration & sync
`src-tauri/src/config` defines `AppConfig` (a list of `ProviderConfig` entries:
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

```
┌────────────────────────────┐        invoke("list_all_books")
│  React + TS frontend (src/) │  ───────────────────────────────►
│  App.tsx renders Vec<Book>  │                                   │
└────────────────────────────┘                                   ▼
                                        ┌───────────────────────────────────┐
                                        │        Rust core (src-tauri)       │
                                        │                                    │
                                        │  commands.rs  list_all_books()     │
                                        │      │  build_providers(AppConfig) │
                                        │      ▼                             │
                                        │  providers::Provider (trait)       │
                                        │   └─ AudiobookshelfProvider (stub)  │
                                        │          │ list_library()          │
                                        │          ▼                         │
                                        │  models::Book (normalized)         │
                                        │                                    │
                                        │  config:: load/save (encrypted*)   │
                                        └───────────────────────────────────┘
                                                     │ direct HTTPS
                                                     ▼
                                        each provider's own API (no Libro server)
```
`*` encryption is a planned boundary, not yet implemented.

## Roadmap (phase order)

1. **Library aggregation** — normalize and merge each provider's library into one
   catalog. *(This skeleton wires the pattern; real Audiobookshelf HTTP is next.)*
2. **Request / acquisition** — request or acquire titles not yet owned (holds,
   downloads).
3. **Plugin / connector system** — harden the `Provider` abstraction, dynamic
   registration, per-connector config UI.
4. **Audiobook playback** — in-app player with progress sync.
5. **Reading** — EPUB reading experience.

## Provider landscape — what's possible

Realistic connector targets (client-usable APIs):
- **Audiobookshelf** — documented REST API; the first real connector.
- **Open Library** — public API for metadata/covers.
- **Hardcover** — public GraphQL API (catalog/social).
- **StoryGraph** — used as a Goodreads replacement for reading data.
- **Send-to-Kindle** — feasible by emailing supported files to a user's
  `@kindle.com` address (the `SEND_TO_KINDLE` capability).

### Deferred / not currently possible
- **Libby / OverDrive** — no public API for third-party clients; can't build a
  supported connector today.
- **Kindle library** — Amazon exposes no API to read a user's Kindle library.
- **Goodreads API** — retired to new developers; use **StoryGraph** / **Hardcover**
  / **Open Library** instead.

## Deliberately deferred implementation (this skeleton)

These are intentionally left as TODOs with clear seams:
- Real Audiobookshelf HTTP calls (`providers/audiobookshelf.rs`).
- Real config encryption + OS keychain + Signal-style backup blob (`config`).
- Audio playback and EPUB reading.
- De-duplication of items across providers via `identifiers`.
