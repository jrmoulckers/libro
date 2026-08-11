# AGENTS.md — libro

Product-specific operating guide for `jrmoulckers/libro`, a member repo of **JRM Studio**.

This file extends the shared studio base guide. The studio sync engine injects that base
between a pair of `studio:base:start` / `studio:base:end` HTML-comment markers below;
everything outside those markers is product-local and is never touched by the sync. Do not
hand-edit inside the managed block — edit the canonical copy in `jrmoulckers/.github` instead.

> **Do not write the two marker strings in their literal HTML-comment form anywhere in this
> file except as the real block delimiters.** The engine locates the managed region with a
> plain regex over the raw file, so prose that quotes both literals — even inside backticks —
> forms a phantom empty block, and the first sync then flags `AGENTS.md` as locally modified
> and skips it.

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
| Lint / format   | `@jrmoulckers/eslint-config` + `@jrmoulckers/prettier-config` (see below) |
| Type-check      | `svelte-check` for the app, `tsc` for config files; `@jrmoulckers/tsconfig` |
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

Rules that restate a ratified engineering principle are cited rather than copied, per
[ADR-0003](https://github.com/jrmoulckers/.github/blob/main/docs/architecture/0003-four-authority-topology.md).
Resolve any `ENG-*` ID through
[`principles/index.json`](https://github.com/jrmoulckers/engineering/blob/main/principles/index.json).

1. **No server tier, ever.** Do not add an API route, a Node server, SSR, or a
   backend-for-frontend. If a feature seems to need one, it needs a different design. This is a
   durable constraint recorded here under
   [`ENG-ARCH-003`](https://github.com/jrmoulckers/engineering/blob/main/principles/architecture/boundaries-and-contracts.md).
2. **No secrets in the repo or the bundle.**
   [`ENG-SEC-001`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/security-and-privacy.md)
   already forbids secrets in source, artifacts, logs, and clients, and
   [`ENG-WEB-001`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
   makes client-visible configuration untrusted. libro is stricter only because it has nowhere
   to inject one at runtime: a feature requiring a private API key is out of scope by
   construction, so third-party integrations must work with public endpoints or with
   user-supplied credentials held in device storage.
3. **User content stays on the device.** Book files, audio, covers, positions, and highlights
   are personal data under
   [`ENG-SEC-008`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/security-and-privacy.md),
   and the device's store is their system of record under
   [`ENG-LOCAL-001`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md).
   libro-specific: there is no destination to send them to, so do not log them and do not add
   analytics that could carry titles or filenames.
4. **Own the client budget.** The budget obligation is
   [`ENG-WEB-003`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
   (separate delivery and runtime budgets),
   [`ENG-PERF-002`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/performance.md)
   (versioned and owned), and
   [`ENG-PERF-003`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/performance.md)
   (dependency cost). libro's numbers: CI enforces 2048 KB of `dist/`, media parsing (EPUB,
   audio, covers) must be dynamically imported and code-split rather than pulled into the entry
   chunk, and every runtime dependency is justified in the PR body. See
   [practices/performance-budgets.md](https://github.com/jrmoulckers/engineering/blob/main/practices/performance-budgets.md).
5. **Offline is a feature, not a fallback.** This is
   [`ENG-LOCAL-004`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)
   (start with zero external-service configuration; degrade unavailable optional services to
   explicit local behaviour) plus
   [`ENG-WEB-002`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
   (detect optional browser capabilities before use). libro-specific: reserve layout space so
   loading states don't cause CLS.
6. **Accessibility is a gate.** WCAG 2.2 AA. Native elements first; media players need full
   keyboard control and correctly labelled transport controls. Honor `prefers-reduced-motion`
   — the tokens already zero durations; don't reintroduce motion that bypasses that.
   [`ENG-PERF-009`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/performance.md)
   additionally forbids trading accessibility away for performance.
7. **Colocate tests.** `src/lib/foo.ts` → `src/lib/foo.test.ts`. A libro convention; the
   obligation it serves is
   [`ENG-TEST-003`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/testing.md)
   (regress at the narrowest authoritative boundary).

Sync behaviour in `src/lib/sync/` is governed by
[`ENG-LOCAL-002`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)
(one narrow provider contract; core local operation never waits on an account, provider, or
network) and
[`ENG-LOCAL-003`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)
(ordering, tombstone, concurrency, and merge rules are declared and tested per synchronized
type). See
[practices/local-first-sync.md](https://github.com/jrmoulckers/engineering/blob/main/practices/local-first-sync.md).

## Shared engineering configuration

Lint, format, and TypeScript settings are **not** authored here. They come from
[`jrmoulckers/engineering`](https://github.com/jrmoulckers/engineering) via GitHub Packages:

| Package | Consumed as |
| --- | --- |
| `@jrmoulckers/eslint-config` | `svelteConfig()` in `eslint.config.js` |
| `@jrmoulckers/prettier-config` | re-exported from `prettier.config.js` |
| `@jrmoulckers/tsconfig` | `vite-app.json` / `vite-node.json` in `tsconfig.*.json` |

Unlike `@jrm/tokens`, these are real registry dependencies, not sync-vendored files. `.npmrc`
points the `@jrmoulckers` scope at `https://npm.pkg.github.com` and holds **no token**; supply
a `read:packages` credential out of band. See
[docs/adopting.md](https://github.com/jrmoulckers/engineering/blob/main/docs/adopting.md).

Do not restate a shared rule in a local override. Genuinely libro-specific lint rules belong in
`svelteConfig({ extend: [...] })`; a rule that is wrong for every repo belongs upstream.

Floors are `eslint-config@^0.8.0`, `tsconfig@^0.3.0`, `prettier-config@^0.3.0`. **On a `0.x`
package a caret permits patch updates only**, so `^0.2.0` resolves to `>=0.2.0 <0.3.0` and can
never reach 0.3.0 — a too-low floor is a silent install-time failure, not a lint-time one. The
`eslint-config` floor is specifically install-time here: libro runs ESLint 10.8.0, and the peer
was `^9.0.0` through 0.3.0. Package versions track independently of the engineering repo's own
tags, so a repo tag is never an actionable npm specifier.

The `typescript` peers deliberately differ between the two packages — **do not align them.**
`@jrmoulckers/tsconfig` declares `^5.5.0 || ^6.0.0 || ^7.0.0`, while `@jrmoulckers/eslint-config`
declares `>=5.5.0 <6.1.0`, because it depends on `typescript-eslint`, whose own peer stops below
6.1. libro runs TypeScript 6.0.3, which satisfies both, so the split does not bite here yet — but
a future move to TypeScript 7 means adopting `tsconfig` and holding `eslint-config` back, not
widening either. As of `prettier-config@0.3.0` every declared peer accommodates libro's
toolchain: `prettier-plugin-svelte` is now `^3.2.0 || ^4.0.0` against our 4.1.1.

### Type-aware linting stays off

`svelteConfig()` is called with no options, so the preset's default `recommended` rule set
applies and no file needs to resolve to a TypeScript project. `strictTypeChecked: true` was
measured against libro at `eslint-config@0.6.0`: it reports **71 findings across 18 files**,
almost all discipline rather than soundness — 27 `require-await`, 20
`prefer-promise-reject-errors`, and **zero** `no-floating-promises`. Adopting it is a deliberate
future change with a real remediation cost, not a free upgrade, and it additionally needs an
upstream fix (see below) before it will run at all here.

### Config files are not type-checked, and that is currently forced

`eslint-config` ships type declarations from 0.8.0, so `svelteConfig()`'s options would be
option-checked if `tsconfig.node.json` enabled `allowJs` + `checkJs`. Both were measured as
working — a valid config compiles clean, and `svelteConfig({ env: 'nonsense' })` is correctly
rejected. libro does **not** enable them, because `@jrmoulckers/prettier-config` ships no
declarations, so `prettier.config.js` fails with `TS7016` under `checkJs`. Re-verified at
`prettier-config@0.3.0`, which widened a peer but shipped no `.d.ts` and no `types` condition, so
the blocker is unchanged. Dropping that one file
from `include` to buy the check would leave it unchecked by *both* `tsc` projects, which is a
worse trade than not opting in. Revisit once `prettier-config` ships types; do not paper over it
with a local `declare module` shim, which would assert a contract libro does not own.

Note the failure is silent today: `tsconfig.node.json` lists the three `.js` config files in
`include`, but with `allowJs` off `tsc` skips them without a word. Verified with `--listFiles`.

## Deviations from the shared principles

Recorded explicitly so reviewers and agents don't treat them as oversights. Each exception
cites the obligation it is an exception to, by stable ID, in the authority that owns it —
the legacy `principles/<realm>.md` files these bullets used to name were deleted from
`jrmoulckers/studio` when its 192 legacy principles were redistributed, so those citations no
longer resolve. Successors below come from `principles/migration-ledger.json` in
`jrmoulckers/studio`.

- **`ENG-API-001`–`ENG-API-004` are vacuous here.** libro has no server and no owned API, so
  there is no request to parse into a typed contract, no server-side authorization decision,
  and no service dependency to bound.
- **`ENG-DATA-001` and `ENG-DATA-003` bind, and are not deviations.** libro has no *server*
  database, but `ENG-DATA-001` governs any durable store and schema — the planned IndexedDB
  library index is exactly that, so it needs one owner, invariants enforced in the data model,
  and reviewed forward-safe migrations between versions. `ENG-DATA-003` (minimize at
  collection; implement retention, export, and erasure) likewise applies to on-device library
  data. Device persistence is additionally governed by `ENG-LOCAL-001`, which makes the
  device's durable store the system of record and requires portable data stay exportable.

  `ENG-DATA-003` admits those mechanisms **"only from an explicit authorized obligation"** —
  Engineering will not let a retention or erasure mechanism exist without a Product obligation
  behind it. Nothing attaches yet, because the store does not exist. **When the IndexedDB index
  is built**, its retention and terminal-disposition behavior must trace to `PROD-COMP-005`
  (purpose-linked retention period, start trigger, terminal disposition, and the consequence of
  deletion or consent withdrawal), and any export or erasure surface must trace to
  `PROD-COMP-003` (promised access, correction, export, deletion, and opt-out behavior per data
  category). Build the store without those citations and the mechanism is unauthorized, not
  merely undocumented.

  `ENG-SEC-008` is complementary, not redundant: `ENG-DATA-003` obligates the *mechanism*,
  `ENG-SEC-008` obligates the *auditable lifecycle evidence* that the mechanism ran. Cite both.
- **`ENG-INT-001`–`ENG-INT-005` have no boundary to govern.** libro owns no service seam. If a
  third-party integration is ever added it must satisfy `ENG-INT-001`–`ENG-INT-004` in the
  client; `ENG-INT-005` (third-party credentials behind a server-side proxy) is unsatisfiable
  by construction, which is exactly why a feature needing a private API key is out of scope.
- **`ENG-WEB-003` applies in full; its legacy SSR guidance does not survive.** The old
  `principles/frontend.md` §6 was carried forward as `ENG-WEB-003` (delivery and runtime
  budgets) by ledger entry `studio-legacy:frontend:6`, disposition `reference`. The "prefer
  server rendering / static where the framework allows" line was an in-practice note under
  that heading and has **no successor** — no ratified Engineering principle mentions server
  rendering. So there is nothing left to deviate from: libro is a static client-rendered SPA,
  and LCP is judged against the SPA shell, not an SSR baseline. The budget obligation itself
  (`ENG-WEB-003`, plus `ENG-PERF-003` on dependency cost) binds us normally.
- **`ENG-SEC-001`, `ENG-SEC-002`, `ENG-SEC-005`, `ENG-SEC-006`, `ENG-SEC-008` bind in full;
  `ENG-SEC-003`, `ENG-SEC-004`, `ENG-SEC-007` have a client-only reading.** The entire threat
  surface is the client bundle and device storage, so there is no server-side default-deny
  decision, no fail-closed service configuration, and the only trust boundary to model is
  untrusted file/network input reaching the browser. `ENG-SEC-001` is stricter here, not
  looser: with no server tier there is nowhere to inject a secret at runtime.
- **`reusable-perf-budget`'s Lighthouse assertions are deliberately deferred**, not overlooked.
  `url: ''` makes the workflow self-skip Lighthouse; the bundle-size budget (2048 KB) still runs
  on every push and PR. This is a **known gap against `ENG-PERF-002`** (budgets must be
  versioned, owned, and method-specific) and against `ENG-PERF-001` (claims need a reproducible
  measurement): the size half is enforced, the field-metric half is not yet measurable. Set
  `url:` in `.github/workflows/ci.yml` once a host is chosen, and `lhci-min-performance` /
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

## Authorities

Product obligations and outcomes are defined in
[`jrmoulckers/product`](https://github.com/jrmoulckers/product) and consumed **by reference,
never by copy**. Cite an obligation by its stable ID (for example `PROD-REL-001`), and pin to a
commit SHA when the exact wording matters. Roadmaps, metrics, experiments, and compliance
evidence live here in libro and cite the obligation they satisfy.

The other three authorities work the same way: engineering mechanisms and evidence in
[`jrmoulckers/engineering`](https://github.com/jrmoulckers/engineering) (`ENG-*`), design and
interface in [`jrmoulckers/studio`](https://github.com/jrmoulckers/studio) (`STUDIO-*`), and
governance, automation, and shared agent assets in
[`jrmoulckers/.github`](https://github.com/jrmoulckers/.github) (`GH-*`). Never copy principle
text into this repository — cite the ID. Where libro cannot satisfy an obligation, record it
under [Deviations](#deviations-from-the-shared-principles) with the ID it excepts.

**Read the principle before claiming it does not apply.** Citing an ID needs only the ID;
asserting that an obligation is *vacuous* here is a stronger claim and needs its text. The
legacy→successor ledger in `jrmoulckers/studio` tells you which principle succeeded a legacy
one — it does not tell you the successor's scope, and the realms were re-cut during the
migration. `ENG-DATA-*`, for instance, is scoped by durability, not by tier, so it binds a
pure-client product that has no server database. Inheriting a legacy realm's framing is how a
false exemption gets written down, and a false exemption is most dangerous for a component that
does not exist yet, because nothing contradicts it until the design is already settled.

`jrmoulckers/engineering` is consumed in three layers, all pinned at `v0.1.0`:

| Layer | What it gives libro | How it arrives |
| --- | --- | --- |
| [`principles/`](https://github.com/jrmoulckers/engineering/blob/main/principles/README.md) | 66 ratified `ENG-*` rules | cited by ID; resolve via [`principles/index.json`](https://github.com/jrmoulckers/engineering/blob/main/principles/index.json) |
| [`practices/`](https://github.com/jrmoulckers/engineering/blob/main/practices/README.md) | technique for satisfying them | linked by URL |
| `packages/` | executable enforcement | GitHub Packages (see [Shared engineering configuration](#shared-engineering-configuration)) |

Practices state no new rules — every normative sentence in one cites the `ENG-*` ID it derives
from, so cite the principle, not the practice, when you need an obligation.

## Not built yet

Routing and a UI shell beyond the current single-view `App.svelte`. The EPUB reader, audio
player, provider adapters (OPDS, Audiobookshelf), metadata enrichment, plugin engine, the
IndexedDB stores, the sync lanes, and the PWA service worker all exist under `src/lib/` with
colocated tests.

<!-- studio:base:start -->
<!-- synced from jrmoulckers/.github — canonical source; do not edit here -->

# AGENTS.md — JRM Studio base operating guide

This file tells an AI agent (GitHub Copilot, Codex, Claude, and others) how to work safely and
effectively across **JRM Studio** repositories. It is the shared floor. **Each product repo
extends it** with its own root `AGENTS.md` that adds product-specific stack, paths, and rules —
product rules layer on top of, and may override, the defaults here.

> This file lives in the canonical `jrmoulckers/.github` backbone repo. It is distributed to
> product repos by the studio sync tool; edit the canonical copy here, not the copies.

## What JRM Studio is

A family of independent product repositories (`jrm-recipes`, `score-king`, `finance`, and more)
that share DNA — work practice, AI agents/skills, community-health files, and reusable CI —
through this backbone repo and `@jrm` npm packages. Products stay independent; the shared layer
keeps them consistent.

## Golden rules

1. **Never commit secrets.** Real values live only in git-ignored files. In tracked files use
   `${VARS}` or placeholders (`YOUR_API_KEY_HERE`) and ship a `.env.example`. If you find a
   secret that would be committed, stop and flag it.
2. **Issue-first, PR-always.** Every change references an issue and lands as a PR. A task that
   ends at a local commit is **incomplete**. Read-only research does not need an issue when it makes
   no repository change; the issue requirement begins before the first change.
3. **Stay in scope.** Make surgical, intentional edits. Don't reformat or "clean up" unrelated
   code. Don't work outside the repository root.
4. **Document decisions.** Non-trivial structural or design choices get an ADR in
   `docs/architecture/` (or the product's ADR location).
5. **When unsure, ask.** Prefer a short clarifying question over a guess that touches
   security, data, or infrastructure.

## Core principles

1. **Privacy first** — treat user data as confidential by default; never log or transmit it in
   plain text.
2. **Accessibility** — UI meets WCAG 2.2 AA minimum: semantic elements, screen-reader support,
   reduced-motion and high-contrast preferences.
3. **Security** — follow OWASP guidance; validate and sanitize inputs; never hardcode secrets.
4. **Transparency** — capture significant trade-offs in commit messages and PR descriptions.
5. **Conventional commits** — `type(scope): description (#N)` (`feat`, `fix`, `docs`, `style`,
   `refactor`, `test`, `chore`, `ci`, `perf`).

## Definition of Done — not complete until ALL gates pass

| Gate | Verification |
| --- | --- |
| **Lint & format** | The repo's lint/format check passes with no errors. |
| **Type-check** | Static type-check passes (where the stack has one). |
| **Tests** | Affected unit/integration tests pass. |
| **Build** | The affected app/package builds. |
| **PR open & green** | A PR is open against the default branch with CI green. |
| **No conflicts** | The PR is `MERGEABLE` (not `DIRTY`/`BEHIND`). |
| **Merged** | The PR is merged once the quality gate passes (unless a documented blocker prevents it). |

Run the repo's own pre-push checks before every push (each product repo documents the exact
commands). Merge conflicts carry the same weight as red CI — resolve them before merging.

## Issue-First Development

1. Every change references a GitHub issue — create one first if none exists.
2. Work on a feature branch (or worktree); never commit directly to the default branch.
3. Commit messages include the issue reference: `type(scope): description (#N)`.
4. Push your feature branch, then open a PR against the default branch with `Closes #N`.
5. Verify the PR exists, then monitor CI until it is green **and** the PR is `MERGEABLE`.
6. Land the work: self-merge your own PR once the quality gate passes. A change left only on a
   side branch is not done. If a real blocker prevents merge, leave one green, `MERGEABLE` PR
   with a `## Needs Human Action` note.

## Coding standards

- Write clear, self-documenting code; comment only when intent isn't obvious.
- Prefer small, focused functions, modules, and PRs.
- Write tests alongside new code (unit tests for logic; integration tests for I/O and APIs).
- Use each language's conventional naming; document public APIs.

## What NOT to do

- Do NOT commit secrets, API keys, tokens, or credentials.
- Do NOT add dependencies without documenting why.
- Do NOT bypass linters, formatters, or CI checks.
- Do NOT ship placeholder implementations without a clearly marked `// TODO:`.
- Do NOT make changes outside the scope of the assigned task.

## Tooling (MCP)

Pinned MCP servers are declared in `agency.toml`. Intrinsically bounded Context7 documentation and
sequential-thinking tools are enabled by default; Playwright browser automation and persistent
memory are documented, pinned opt-ins until the consuming host's tool-filter enforcement is
verified. Product repos may define a narrower local runtime policy.

## Human-Gated Operations (MANDATORY)

These apply to **all** AI tools in every studio repo. Pushing feature branches and creating PRs
is **required and auto-approved** — stopping at a local commit to ask permission is a workflow
violation. The operations below, however, require explicit human approval.

**1 — Git remote.** Auto-approved: push/rebase your **own** feature branch, `fetch`,
`force-with-lease` on your own branch to resolve a rebase/conflict, read-only git.
Gated/forbidden: pushing to `main`/release branches, plain `git push --force`, force-with-lease
on shared branches, remote/merge reconfiguration.

**2 — Pull requests.** Auto-approved on **your own** PRs: create, review, request changes,
merge once the quality gate passes (CI green AND `MERGEABLE`). Gated: merging, approving,
closing, or dismissing reviews on a PR you did **not** author; merging while CI is red or the PR
conflicts.

**3 — Remote platform.** Auto-approved: routine triage labels. Gated: closing/reopening/deleting
issues, changing gating labels (`blocked`, `security`, `breaking-change`), and any repo-settings,
branch-protection, secrets, deployment, or `gh api` write.

**4 — Outside project boundary.** Never read, write, or execute outside the repository root, and
never modify system configuration or install global tools.

**5 — Destructive file ops.** No recursive/bulk/wildcard deletion; name each file to remove and
explain why. Never overwrite a file without reading it first.

**6 — Publishing & distribution.** No `npm publish`, image pushes, store submission, or deploy
scripts. Prepare the release and hand the final publish to a human.

**7 — Secrets & credentials.** Never create/read real secret files, access OS keychains, generate
real keys, or echo secret-bearing env vars. Use `.env.example` placeholders.

**8 — Destructive database ops.** No `DROP`/`TRUNCATE`/unqualified `DELETE`/destructive `ALTER`,
no restores, no pointing connection strings at production. Write reversible migrations for a human
to review and run.

If a task needs a gated operation: **stop, state what and why, and wait for approval.** Never work
around these restrictions. If no human is available, complete everything that is auto-approved,
then leave a clear `## Needs Human Action` note.

## Nested guides

Scope-specific rules live alongside the code — read the relevant one before working in that area:

- Each product repo's root `AGENTS.md` — stack, paths, and product-specific rules.
- `agents/*.agent.md` in this backbone, materialized as `.github/agents/*.agent.md` in consumers —
  role definitions and boundaries. Consumer copies are generated; product-specific stack/path/risk
  overlays belong in the product's root `AGENTS.md` or scoped instructions.
- `skills/<name>/SKILL.md` in this backbone is canonical; opted-in consumers read the generated,
  upstream-owned `.github/skills/<name>/SKILL.md` materialization.
- `instructions/*.instructions.md` in this backbone is canonical; opted-in consumers read generated,
  upstream-owned `.github/instructions/*.instructions.md` copies. Root/local `AGENTS.md` and
  more-specific scoped instructions override shared defaults without relaxing mandatory human
  gates.
<!-- studio:base:end -->
