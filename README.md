# Libro

A cross-platform, **pure-client** media hub for books, audiobooks, and your personal library.

Libro aggregates your books and audiobooks across many providers into one normalized catalog — with no backend server. The app talks directly to each provider's API from your device; configuration stays local and (in a later phase) syncs device-to-device, Signal-style, via an encrypted backup blob you control.

> **Status:** early skeleton. The Audiobookshelf (self-hosted REST), Hardcover
> (official public GraphQL), and LazyLibrarian (self-hosted REST) connectors are
> real clients — live end-to-end verification is pending a running server / a
> user-supplied API key. Libby ships as a deep-link-only placeholder. Metadata
> enrichment via **Open Library** and **Google Books** (official public APIs) is
> real and live-verified. See
> [`ARCHITECTURE.md`](./ARCHITECTURE.md)
> for the design, roadmap, and legal boundaries.

## Stack

- **App shell:** [Tauri v2](https://tauri.app/) — one codebase targeting desktop (Windows/macOS/Linux) and mobile (iOS/Android).
- **Frontend:** React + TypeScript + [Vite](https://vite.dev/).
- **Core / business logic:** Rust (in the Tauri core).
- **Architecture:** pure client — no backend, no database server, no cloud storage.

## Project layout

```
.
├─ Cargo.toml               # Rust workspace (members: core, src-tauri)
├─ index.html, vite.config.ts, tsconfig*.json   # frontend build config
├─ src/                     # React + TypeScript UI
│  ├─ App.tsx               # single page: calls `list_books_by_provider`, renders the catalog
│  └─ types.ts              # TS mirror of the Rust domain model
├─ core/                    # `libro-core`: pure-client logic (no Tauri/GUI)
│  └─ src/
│     ├─ models/            # normalized domain model (Book, MediaType, Progress)
│     ├─ providers/         # Provider trait + connectors (Audiobookshelf, Hardcover, LazyLibrarian, Libby)
│     ├─ metadata/          # MetadataProvider trait + enrichment (Open Library, Google Books)
│     └─ config/            # local, encrypted-at-rest config (boundary only)
└─ src-tauri/               # `libro`: the Tauri shell (depends on libro-core)
   ├─ src/
   │  ├─ commands.rs        # `list_all_books` / `list_books_by_provider` aggregation
   │  └─ lib.rs / main.rs   # Tauri entry points
   ├─ tauri.conf.json
   └─ capabilities/
```

## Prerequisites

- **Node.js** 18+ and npm.
- **Rust** (stable) via [rustup](https://rustup.rs/).
- Platform build dependencies for Tauri — see the [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/).
  - On **Windows** the standard target is `x86_64-pc-windows-msvc`, which needs the **Visual Studio C++ Build Tools** (MSVC toolset + Windows SDK) and [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/).

## Develop

```bash
npm install          # install frontend deps
npm run tauri dev    # run the desktop app with hot reload
```

Frontend only (no native shell):

```bash
npm run dev          # Vite dev server on http://localhost:1420
```

## Build

```bash
npm run build        # type-check + build the frontend (tsc && vite build)
npm run tauri build  # produce native installers/binaries
```

You can type-check and test the Rust core without bundling:

```bash
cargo check --workspace       # type-check both crates
cargo test -p libro-core      # run the domain/connector unit tests
```

## License

TBD.
