# AGENTS.md — libro

Product-specific operating guide for `jrmoulckers/libro`, a member repo of **JRM Studio**.

This file extends the shared studio base guide. The studio sync engine injects that base
between `<!-- studio:base:start -->` / `<!-- studio:base:end -->` markers below; everything
outside those markers is product-local and is never touched by the sync. Do not hand-edit
inside the managed block — edit the canonical copy in `jrmoulckers/.github` instead.

## What libro is

A **cross-platform, pure-client media hub** for books, audiobooks, and your personal library.
Browse, organize, and play a personal collection across desktop and mobile browsers.

**Pure-client is a hard architectural constraint, not a phase.** There is no server tier, no
API we own, and no backend deployment. Everything runs in the browser: the library index,
metadata, reading/listening position, and playback all live on the user's device.

## Stack

| Concern         | Choice                                                                     |
| --------------- | -------------------------------------------------------------------------- |
| Framework       | Svelte 5 (runes)                                                           |
| Build / dev     | Vite                                                                       |
| Language        | TypeScript (strict)                                                        |
| Package manager | **pnpm** (`packageManager` field is authoritative; do not use npm or yarn) |
| Tests           | Vitest (jsdom)                                                             |
| Lint / format   | ESLint flat config + Prettier (`prettier-plugin-svelte`)                   |
| Type-check      | `svelte-check` for the app, `tsc` for config files                         |
| Output          | Static `dist/` — deployable to any static host or CDN                      |
| Design tokens   | `@jrm/tokens`, vendored from `jrmoulckers/studio` (see below)              |

This mirrors `score-king`'s Svelte/Vite PWA stack deliberately, so the two products share
the same review, CI, and token surface.

## Commands

```bash
pnpm install          # install (CI uses --frozen-lockfile)
pnpm dev              # dev server
pnpm build            # production build -> dist/
pnpm preview          # serve the built output
pnpm lint             # eslint .
pnpm format           # prettier --write .
pnpm format:check     # prettier --check .   (CI gate)
pnpm typecheck        # svelte-check + tsc
pnpm test             # vitest run
```

**Pre-push gate — run all of these before pushing:**
`pnpm lint && pnpm format:check && pnpm typecheck && pnpm test && pnpm build`.
These are exactly the commands `.github/workflows/ci.yml` runs.

## Layout

```
.
├── index.html               # Vite entry
├── src/
│   ├── main.ts              # mount
│   ├── App.svelte
│   ├── app.css              # global stylesheet; token import lives here
│   └── lib/                 # domain logic + colocated *.test.ts
├── vendor/@jrm/tokens/      # SYNC-OWNED, arrives via chore(sync) PR — never hand-write
├── .github/workflows/ci.yml # product-owned; calls the backbone reusable workflows
└── dist/                    # build output (git-ignored)
```

The app lives at the **repo root** (score-king parity), not under `apps/`. That is what makes
the manifest's default `tokens.targetPath` (`vendor/@jrm/tokens`) correct for this repo — no
per-member override is needed, unlike `finance`.

## Design tokens (`@jrm/tokens`)

Tokens are **not** installed from a registry. The studio sync engine mirrors
`jrmoulckers/studio`'s committed `packages/tokens/dist/` tree into `vendor/@jrm/tokens/` and
opens a `chore(sync)` PR. Those files are generated and provenance-stamped: **never edit,
copy, or hand-write anything under `vendor/`** — doing so trips the engine's drift detection
and the file is then skipped on every subsequent sync.

Once the first sync PR lands, uncomment the import at the top of `src/app.css`:

```css
/* src/app.css — relative to this file */
@import '../vendor/@jrm/tokens/css/default/index.css';
```

Repo-root-relative, that path is **`./vendor/@jrm/tokens/css/default/index.css`**. Vite
resolves the relative form from `src/` cleanly; no alias, no `resolve.alias` entry, and no
`tokens.targetPath` override in `studio.config.json` are required.

### `@jrm/tokens` is the ONLY `@jrm` package libro gets

The studio is registry-free, and the sync engine vendors **only** the token `dist/` tree.
`@jrm/tailwind-preset`, `@jrm/eslint-config`, `@jrm/tsconfig`, and `@jrm/prettier-config` are
**not** synced into member repos and **cannot resolve here**. Studio's README shows
`presets: [require('@jrm/tailwind-preset')]`, `"extends": "@jrm/tsconfig/svelte.json"`, and
`"prettier": "@jrm/prettier-config"` — those examples do **not** apply to libro. Our
`eslint.config.js`, `.prettierrc.json`, and `tsconfig*.json` inline the equivalents locally,
and must stay that way. Adding an `@jrm/*` dependency other than the vendored tokens will fail
`pnpm install --frozen-lockfile` in CI.

Rules that follow from studio's frontend principles:

- Style with semantic custom properties — `var(--color-surface)`, `var(--color-text)`,
  `var(--radius-md)`, `var(--shadow-lift)`. Never inline a hex, px radius, shadow, or duration.
- Bind to semantic names, not primitives (`--color-primary`, not `--color-violet-500`) so a
  theme swap re-flows the UI.
- Switch appearance only via `document.documentElement.dataset.theme`
  (`dark` / `dark-oled` / `high-contrast`; light = attribute removed). Apply the persisted
  choice before first paint to avoid a theme flash. Never ship a second stylesheet per mode.
- Reach for the typed JS export only where CSS cannot go (canvas, waveform rendering).

Until the first sync lands, `src/app.css` carries only structural, value-free CSS. Do not
introduce placeholder color/spacing literals "for now" — they become permanent drift.

## Product-specific rules

1. **No server tier, ever.** Do not add an API route, a Node server, SSR, or a
   backend-for-frontend. If a feature seems to need one, it needs a different design.
2. **No secrets in the repo or the bundle.** There is no server to hold them. A feature that
   requires a private API key is out of scope by construction. Third-party integrations must
   work with public endpoints or user-supplied credentials held in device storage only.
3. **User content stays on the device.** Book files, audio, covers, positions, and highlights
   are private user data. Do not send them anywhere, do not log them, do not add analytics
   that could carry titles or filenames.
4. **Own the client budget.** CI enforces a 2048 KB `dist/` budget. Media parsing (EPUB,
   audio, covers) must be dynamically imported and code-split, never pulled into the entry
   chunk. Justify every runtime dependency in the PR body.
5. **Offline is a feature, not a fallback.** Anything added must degrade sensibly with no
   network. Reserve layout space so loading states don't cause CLS.
6. **Accessibility is a gate.** WCAG 2.2 AA. Native elements first; media players need full
   keyboard control and correctly labelled transport controls. Honor `prefers-reduced-motion`
   — the tokens already zero durations; don't reintroduce motion that bypasses that.
7. **Colocate tests.** `src/lib/foo.ts` → `src/lib/foo.test.ts`.

## Deviations from the shared principles

Recorded explicitly so reviewers and agents don't treat them as oversights:

- **`principles/backend.md` and `principles/middleware.md` do not apply.** libro has no
  server, no database, and no service boundary. Persistence is browser storage.
- **`principles/frontend.md` §6, "prefer server rendering / static where the framework
  allows"** — libro is a static client-rendered SPA. The "static" half applies (the build is
  fully static); SSR does not exist here. Judge LCP against the SPA shell, not an SSR baseline.
- **`principles/security.md` browser posture applies in full, but its server-side controls do
  not.** The entire threat surface is the client bundle and device storage.
- **`reusable-perf-budget`'s Lighthouse assertions are deliberately deferred**, not overlooked.
  `url: ''` makes the workflow self-skip Lighthouse; the bundle-size budget (2048 KB) still runs
  on every push and PR. This is a **known gap pending a hosted preview URL** — set `url:` in
  `.github/workflows/ci.yml` once a host is chosen, and `lhci-min-performance` /
  `lhci-min-accessibility` then default to 90 / 95.
- **Deploy previews use the `artifact` provider**, not a hosting provider, for the same reason.

## Sync-engine boundaries — do not hand-author these

The studio sync engine owns the files below. Creating or editing them locally produces
immediate false drift and the engine will then skip them permanently:

- `.github/agents/`, `.github/skills/`, `.github/prompts/`, `.github/instructions/`
- `.studio-sync.lock.json`
- `vendor/@jrm/tokens/**`
- the managed block inside this file

Community-health files (`CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, issue and PR
templates) are **inherited automatically** from `jrmoulckers/.github` by GitHub — do not add
local copies.

`.github/workflows/ci.yml` is the opposite case: reusable workflows are native, so the engine
reports them but never writes them. That file is ours to own and edit.

## Not built yet

Routing, the IndexedDB library index, EPUB rendering, audio playback and position sync, PWA
service worker and offline strategy, and file/OPDS import. The current `src/` tree is a
minimal skeleton that exists so the CI gates are real.
