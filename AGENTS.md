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
| Lint / format   | ESLint flat config + Prettier (shared config from `jrmoulckers/engineering`) |
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
├── docs/architecture/       # ADRs — required by ENG-ARCH-003 before a tradeoff is durable
├── config/engineering/      # VENDORED from jrmoulckers/engineering at a pinned ref;
│                            #   byte-identical, SHA-256-pinned — refresh via the script only
├── scripts/vendor-configs.mjs  # fetches the above; committed so a refresh is reproducible
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
`"prettier": "@jrm/prettier-config"` — those examples do **not** apply to libro. libro's
equivalents come from `jrmoulckers/engineering` instead (see [Shared engineering
configuration](#shared-engineering-configuration)), and must stay that way. Adding an `@jrm/*`
dependency other than the vendored tokens will fail `pnpm install --frozen-lockfile` in CI.

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
   backend-for-frontend. If a feature seems to need one, it needs a different design. No ratified
   principle forbids a server tier — this constraint is libro's own, and it is recorded as
   [ADR-0001](docs/architecture/0001-pure-client-architecture.md) because
   [`ENG-ARCH-003` (Durable decisions)](https://github.com/jrmoulckers/engineering/blob/main/principles/architecture/boundaries-and-contracts.md)
   requires an ADR before a tradeoff may be treated as a durable constraint.
2. **No secrets in the repo or the bundle.**
   [`ENG-SEC-001` (Secret lifecycle)](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/security-and-privacy.md)
   already forbids secrets in source, artifacts, logs, and clients, and
   [`ENG-WEB-001` (Browser trust seam)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
   makes client-visible configuration untrusted. libro is stricter only because it has nowhere
   to inject one at runtime: a feature requiring a private API key is out of scope by
   construction, so third-party integrations must work with public endpoints or with
   user-supplied credentials held in device storage.
3. **User content stays on the device.** Book files, audio, covers, positions, and highlights
   are personal data under
   [`ENG-SEC-008` (Privacy-minimizing lifecycle evidence)](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/security-and-privacy.md),
   and the device's store is their system of record under
   [`ENG-LOCAL-001` (Local durable ownership)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md).
   libro-specific: there is no destination to send them to, so do not log them and do not add
   analytics that could carry titles or filenames.
4. **Own the client budget.** The budget obligation is
   [`ENG-WEB-003` (Measured foreground performance)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
   (separate delivery and runtime budgets),
   [`ENG-PERF-002` (Versioned performance budgets)](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/performance.md)
   (versioned and owned), and
   [`ENG-PERF-003` (Minimal package surface)](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/performance.md)
   (dependency cost). libro's numbers: CI enforces 2048 KB of `dist/`, media parsing (EPUB,
   audio, covers) must be dynamically imported and code-split rather than pulled into the entry
   chunk, and every runtime dependency is justified in the PR body. See
   [practices/performance-budgets.md](https://github.com/jrmoulckers/engineering/blob/main/practices/performance-budgets.md).
5. **Offline is a feature, not a fallback.** This is
   [`ENG-LOCAL-004` (Zero-config safe degradation)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)
   (start with zero external-service configuration; degrade unavailable optional services to
   explicit local behaviour) plus
   [`ENG-WEB-002` (Capability-safe enhancement)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
   (detect optional browser capabilities before use). libro-specific: reserve layout space so
   loading states don't cause CLS.
6. **Accessibility is a gate.** WCAG 2.2 AA. Native elements first; media players need full
   keyboard control and correctly labelled transport controls. Honor `prefers-reduced-motion`
   — the tokens already zero durations; don't reintroduce motion that bypasses that. No ratified
   principle sets an accessibility bar, so WCAG 2.2 AA is libro's own standard. Engineering
   constrains only the tradeoff:
   [`ENG-PERF-009` (Assurance precedence)](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/performance.md)
   forbids accepting a performance change that weakens accessibility.
7. **Colocate tests.** `src/lib/foo.ts` → `src/lib/foo.test.ts`. Where the file sits is a libro
   convention; no ratified principle governs test placement. The separate ratified obligation is
   [`ENG-TEST-003` (Regression boundaries)](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/testing.md)
   — a failing regression test at the narrowest authoritative
   boundary for every new behavior, corrected defect, or changed shared contract. Colocation is
   merely where libro keeps such a test, not what satisfies the obligation.
8. **Domain logic stays framework-free, and the dependency direction is one-way.** No `.ts`
   module under `src/lib/` imports from `svelte` or from a component — the framework edge is the
   `.svelte` files (including the two colocated at `src/lib/player/Player.svelte` and
   `src/lib/reader/Reader.svelte`), which may consume `src/lib/` but must not be the place logic
   lives. The framework-isolation half of
   [`ENG-INT-001` (Thin typed adapters)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/integration-boundaries.md)
   obligates the first part, and
   [`ENG-ARCH-001` (Minimal directed boundaries)](https://github.com/jrmoulckers/engineering/blob/main/principles/architecture/boundaries-and-contracts.md)
   (smallest explicit boundary, dependencies acyclic, each fact in one authoritative home)
   obligates the second. The specific `component → store → lib` shape libro uses is a libro
   choice, not an Engineering rule — cite these two for the *direction*, not for the tiering.

Sync behaviour in `src/lib/sync/` is governed by
[`ENG-LOCAL-002` (Optional sync seam)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)
(one narrow provider contract; core local operation never waits on an account, provider, or
network) and
[`ENG-LOCAL-003` (Declared conflict model)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)
(ordering, tombstone, concurrency, and merge rules are declared and tested per synchronized
type). See
[practices/local-first-sync.md](https://github.com/jrmoulckers/engineering/blob/main/practices/local-first-sync.md).

## Shared engineering configuration

Lint, format, and TypeScript settings come from
[`jrmoulckers/engineering`](https://github.com/jrmoulckers/engineering), over **two different
channels**. Which channel a config uses is decided by whether it owns runtime dependencies, not by
its file format, and the split is recorded upstream in
[ADR-0001](https://github.com/jrmoulckers/engineering/blob/main/docs/architecture/0001-two-channel-config-delivery.md).

| Config | Channel | Adopted |
| --- | --- | --- |
| `@jrmoulckers/tsconfig` | vendored at a pinned ref | yes |
| `@jrmoulckers/prettier-config` | vendored at a pinned ref | yes |
| `@jrmoulckers/eslint-config` | GitHub Packages registry | **no — blocked** |

The reason for the split is that GitHub Packages **authenticates every read, including of a public
package**. Routing the `@jrmoulckers` scope therefore puts a credential in the install path for
every contributor and self-hoster, not just CI — which for a pure-client product distributed to
people who run their own copy is an onboarding regression, and one that per-package visibility does
not fix, because visibility changes authorization and not authentication. `tsconfig` and
`prettier-config` have no runtime dependencies, so they can be fetched and committed directly.
`eslint-config` cannot: it depends on `@eslint/js`, `typescript-eslint`, `eslint-config-prettier`,
and `globals`, and copying it would hand four version choices back to every repo — exactly the
drift the shared layer exists to remove.

### The vendored half

Files live in `config/engineering/`, are written **byte-identical to upstream** with no generated
header, and are pinned by ref plus a SHA-256 per file in `engineering-configs.lock.json`. Refresh
with the committed script, never by hand:

```bash
gh api repos/jrmoulckers/engineering/releases/latest --jq .tag_name   # resolve, don't guess
node scripts/vendor-configs.mjs <that-tag>
```

**Resolve the tag; never copy a version literal out of upstream docs.** A literal in a document
does not merely go stale — a tag can carry guidance a later release reversed, so pinning to a
number someone wrote down can reintroduce the behaviour the document exists to prevent. Upstream
now writes these recipes with a placeholder for that reason.

`pnpm vendor:check` verifies the tree against the lock. The two severities are deliberate and
worth preserving: **drift exits non-zero** (a generated file was edited or a write was lost — a
local integrity failure), while **a newer upstream release only warns, exit 0**. Failing on
staleness would make pinning automatic in effect, because a red build pressures the next person
into bumping the ref without deciding to accept the change. It runs as the first step of
`pnpm lint`, so drift is caught by CI rather than by memory.

**The staleness notice keys on the repository tag, and the vendored payload does not.** Nine
consecutive refreshes — `v0.15.1` through `v0.18.0` — each reported `0 file(s) changed content`,
because the upstream tags moved for docs and tooling while `tsconfig` stayed at package version
`0.4.0` and `prettier-config` at `0.3.0` throughout. So the notice has fired nine times and been
right zero times about what it names. To answer *"has my config actually changed"*, compare the
package version rather than the tag:

```bash
gh api repos/jrmoulckers/engineering/contents/versions.json --jq '.content' | base64 -d
git show <pinned-ref>:packages/tsconfig/package.json   # in a checkout of the engineering repo
```

Upstream's `versions.json` records the published version per package and is registry-verified in
their CI, which makes it safe to read at a tag — unlike `packages/<name>/package.json`, which
reports the source tree at that moment and is what has repeatedly caused repos to report
already-fixed defects. Note that for a **vendored** package its `range` field is not actionable:
there is no dependency to pin, only a ref, so read `version` and ignore `range`.

Because the bytes are hashed, **`config/engineering/` is Prettier-ignored**. Reformatting a
vendored file would change its bytes and break the hash, converting a real upstream-drift signal
into an apparent local edit. That the files happen to be Prettier-clean today is luck, not a
guarantee — the ignore is what makes it safe.

Two consequences worth knowing. Vendoring normally costs the version signal a registry provides;
here the lock file supplies it, so a refresh is a reviewable diff and drift is detectable. And
`@tsconfig/svelte` is gone — `tsconfig.app.json` extends the vendored `vite-app.json` instead, and
the `noUnusedLocals` / `noUnusedParameters` libro contributed upstream come back through it.

**The two channels deliver the same bytes, and that is checked rather than assumed.** All six
vendored `tsconfig` files were compared by SHA-256 against the published
`@jrmoulckers/tsconfig@0.4.0` tarball pulled from the registry: identical. So choosing the vendored
channel to avoid the token requirement costs nothing in content — it is purely a delivery
difference.

**The presets are deliberately not supersets, so the swap was diffed option by option rather than
assumed.** Resolving both `extends` chains fully (old: `@tsconfig/svelte@5.0.8`, the version the
lockfile actually pinned) gives 9 differing options for the app project and 13 for the node one.
Three are losses and each is accounted for:

- **`sourceMap`** — dropped deliberately. `@tsconfig/svelte` sets it to place Svelte compiler
  diagnostics correctly, a rationale that predates Svelte 5; upstream documents dropping it.
- **`esModuleInterop`** — dropped on the app project and **not** re-added. Nothing in `src/`
  default-imports a CommonJS module, so `svelte-check` is clean without it. The node project is
  different: it keeps a local `allowSyntheticDefaultImports: true`, which is the one genuine local
  retention in either file.
- **`target`** `ESNext` → `ES2023` — a narrowing, not a loss, and `lib` is now pinned rather than
  inferred.

Everything else is the presets being stricter. `noUncheckedIndexedAccess` is the consequential
one — it is what the accompanying `src/` corrections exist for, and it applies to both projects,
alongside `noImplicitOverride`, `noFallthroughCasesInSwitch`, and
`forceConsistentCasingInFileNames`.
`exactOptionalPropertyTypes` is explicitly `false`, so it is pinned off rather than left to drift.
Do not restore a dropped option because it *looks* missing; an option the preset lacks is a finding
to state, and hoisting one into the shared base breaks every other consumer.

`node.json` is vendored with the rest of the set but deliberately **unextended**. It exists for a
package whose Node executes `.ts` directly, and libro has none: it is a single package with no
workspace, nothing invokes `node` on a `.ts` file or `--experimental-strip-types`, and no import
specifier carries a `.ts` extension. That also makes the `allowImportingTsExtensions` regression
other repos hit structurally impossible here — the option is set **only** in `node.json`, and
libro's pre-migration configs never set it, so there was nothing to lose. Choose the variant per
package, not per repository; if that ever changes, `node.json` is the answer, and it needs
`@types/node` or the first run fails with a `TS2688` that reads like a broken preset.

### The registry half

`eslint.config.js` is still authored locally, because `@jrmoulckers/eslint-config` cannot be
installed: the package is **private**, and that flip is owner-only. `jrmoulckers/engineering` is
itself a public repo, which does not make its packages public — the two settings are independent,
and no registry error distinguishes them, since an anonymous read 401s and a scopeless token 403s
whether the package is public or not.

Read the two codes as different failures rather than degrees of the same one. **`401` is
authentication** — no token, wrong host, or the wrong class of token — and is fixable from here.
**`403 permission_denied: read_package` is authorization**, and no amount of token work resolves
it. The tell is metadata resolving while only the tarball download fails: that means the request
authenticated fine and was refused on access, so the remaining blocker is a grant or a visibility
flip, not the wiring.

**The blocker is CI-only, and narrower than "cannot be installed" suggests.** Probed directly
against `https://npm.pkg.github.com` with a PAT carrying `read:packages`: metadata returns `200`
for all three packages, and the `eslint-config@0.9.0` **tarball also returns `200`** and extracts
to the expected tree. So a developer holding a scoped PAT can install and generate a lockfile
today — the earlier claim that no token here could do so was wrong.

What remains blocked is **CI**, structurally rather than for want of configuration: the packages
are `visibility: private` (confirmed via `gh api users/jrmoulckers/packages/npm/<name>`), and
libro's `GITHUB_TOKEN` is scoped to libro, so it cannot read a private package owned by
`jrmoulckers/engineering` without an explicit grant. No amount of workflow wiring changes that,
which is why the wiring is already correct and still insufficient. Do not "fix" CI by widening a
permission; the grant or the visibility flip is owner-only.

The practical consequence: **generating a lockfile locally is possible but must not be committed
until CI can install**, because a lockfile CI cannot resolve fails every job rather than only the
lint step.

When it is unblocked, adopt at `@jrmoulckers/eslint-config@>=0.10.0 <1.0.0` and replace the local
file with `svelteConfig()`; libro's current rule set is a strict subset, so nothing is lost.

**Keep `eslint-plugin-svelte` in `devDependencies`** — libro already declares it, and must
continue to. Through 0.8.0 the preset declared the five framework plugins as optional peers, on
the belief that `peerDependenciesMeta.optional` prevents installation. It does not: it only
suppresses the missing-peer *error*, while npm 7+ still auto-installs any optional peer it can
resolve, so every consumer received every framework's toolchain (75 MB measured; 36.6 MB after).
From 0.9.0 the ranges sit in a top-level `frameworkPlugins` field that npm ignores, and the
consumer declares its own. Verified against `v0.16.0`: the declared range is
`^2.46.0 || ^3.0.0`, which libro's 3.22.0 satisfies, and `svelte.js` imports the plugin with a
static top-level `import`, so omitting it fails at config load naming the package rather than
degrading silently.

**Write the range that way, not as a caret.** On a `0.x` package a caret permits patch updates
only — `^0.9.0` is `>=0.9.0 <0.10.0` and can never reach 0.10.0 — so a caret floor silently
freezes you on the minor you pinned. The failure is invisible in the worst way: install succeeds,
CI stays green, and you go on reporting defects that were fixed several releases ago, because the
fix is unreachable. An explicit `>=x <1.0.0` is the only form that tracks a pre-1.0 package.

The floor is also **install-time load-bearing**, which is a separate point: libro runs ESLint
10.8.0, and the peer was `eslint: ^9.0.0` through 0.3.0, so too low a floor fails to *resolve*
rather than failing to lint.

**0.10.0 raises the floor again, and for libro it is a correctness fix rather than housekeeping.**
Below it the preset's own `@eslint/js` dependency was capped at `^9.14.0`. That package's major
tracks ESLint's, so an ESLint 10 consumer resolved the ESLint **9** recommended rule set while
every peer range and every gate reported agreement — the preset advertised ESLint 10 support it
did not deliver. libro is exactly that consumer. Verified on the registry at `0.10.0`:
`@eslint/js ^9.14.0 || ^10.0.0`, `eslint-config-prettier ^9.1.0 || ^10.0.0`,
`globals ^15.12.0 || ^16.0.0 || ^17.0.0`. Note the asymmetry worth generalising: a *peer* range
is the consumer's problem and visible in the manifest, whereas a stale *dependency* range is the
preset's own and invisible to everyone — nothing in a consumer repo can observe it, which is why
this survived being green everywhere.

`typescript-eslint` stays at `^8.13.0` and is **not** part of that defect: 8.67.0 is current, the
caret reaches it, and its own `typescript: >=4.8.4 <6.1.0` peer is what the preset's
`>=5.5.0 <6.1.0` cap inherits. Do not "fix" it by widening.

Where a preset defect is reported, quote the **resolved** version — read it from the lockfile or
`node -p "require('@jrmoulckers/eslint-config/package.json').version"`. A pinned range is not an
observation; under a caret freeze the two routinely disagree, and that gap is exactly what makes
the freeze invisible. For the same reason, never read a package's version or peers from a *repo*
tag: `v0.4.0` of the engineering repo shipped `eslint-config` 0.3.0, so the ref resolves cleanly
and answers the wrong question.

One known defect to avoid on adoption: **do not pass `strictTypeChecked: true` to
`svelteConfig()`.** It aborts the entire ESLint run on the first `.svelte` file — the type-checked
rule sets apply unscoped, while the re-disable blocks that rescue `.ts`/`.js` match neither
`.svelte` nor its variants. Re-verified still present at `v0.17.0`. Reported upstream; a consumer
can self-rescue through `extend`, but the plain default is unaffected, so leave it alone.

Do not add the dependency or an `.npmrc` before then: a lockfile that cannot resolve in CI is worse
than no config, and a committed project-level `.npmrc` outranks the user-level one `setup-node`
writes. When it is added it must map **only the scope**, never the default registry —
`registry=https://npm.pkg.github.com/` breaks `pnpm audit` with
`ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS`, because GitHub Packages implements no advisory endpoint and
no token fixes a missing endpoint.

```ini
@jrmoulckers:registry=https://npm.pkg.github.com
```

### Things that stay true across both channels

CI is already wired for the registry half. `.github/workflows/ci.yml` passes `registry-url` and
`registry-scope` to every install-bearing reusable workflow and grants each caller job
`packages: read`, but passes **no `NODE_AUTH_TOKEN`**: at the pinned SHA the workflows resolve it as
`inputs.registry-url != '' && (secrets.NODE_AUTH_TOKEN || github.token) || ''`, so `GITHUB_TOKEN` is
supplied automatically and only on runs that opted into a registry.

**Before re-pinning on advice, compare the callees' blob SHAs, not the commit distance.** The five
call sites sit at `f145727`, which is behind current `main` by several commits — and all five called
workflows are **byte-identical** at both refs, because the intervening commits touch the sync
engine, docs, and a smoke-test workflow libro does not call. A pin exists to freeze *content*, so
"behind by
commits" and "behind by content" are different questions and only the second one matters. Ask the
contents API for each callee's `sha` at both refs; a bare `compare` file list answers it too, but
only if you check it against the call sites rather than skimming it. Re-pinning to a ref whose
callees are identical is churn that can only introduce error.

**`packages: read` is mandatory on the caller**, and it tracks what the callee *declares*, not
whether it installs — `reusable-perf-budget` runs no install and still needs the grant. A caller
`permissions:` block replaces the defaults rather than adding to them, and a called workflow can
never hold a scope its caller lacks, so omitting one fails the run at `startup_failure` with no
readable log. `reusable-ci-lint` additionally needs `pull-requests: read`. `actionlint` does not
model this ceiling, so a green lint is not evidence.

`reusable-security-ci` needs no registry wiring, despite appearances. It has no install step — only
checkout, `setup-node`, and `pnpm audit` — and audit resolves advisory data from the default
registry. Do not add `registry-url` or `packages: read` to that call site.

The `typescript` peers are **deliberately different between the two packages — do not align them.**
`tsconfig` declares `^5.5.0 || ^6.0.0 || ^7.0.0`, while `eslint-config` declares `>=5.5.0 <6.1.0`,
because it depends on `typescript-eslint`, whose own peer stops below 6.1. libro's TypeScript 6.0.3
satisfies both, so the split does not bite yet — but a move to TypeScript 7 means adopting
`tsconfig` and holding `eslint-config` back, not widening either.

Package versions track independently of the engineering repo's own tags, and the skew runs the
direction people do not expect: repo tag `v0.4.0` ships `eslint-config` **0.3.0**. So a repo tag is
not an actionable npm specifier *and* is not a safe ref to verify a package version at. Read
`version` from the same `package.json` you read the peer from.

Do not restate a shared rule in a local override. Genuinely libro-specific lint rules belong in
`svelteConfig({ extend: [...] })`; a rule that is wrong for every repo belongs upstream.

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
- **Only `ENG-INT-005` is a deviation; `ENG-INT-001`–`ENG-INT-004` bind today.** An earlier
  version of this file claimed the whole `ENG-INT-*` family had "no boundary to govern" because
  libro owns no service seam. That was a false exemption, produced by reading the realm's name
  instead of the principles' text.
  [`ENG-INT-001` (Thin typed adapters)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/integration-boundaries.md)
  governs *external input* and *framework* behaviour, not just service seams: libro parses EPUB
  containers, OPF metadata, and OPDS feeds, and isolates framework behaviour by keeping every
  `.ts` module in `src/lib/` free of `svelte` imports. `ENG-INT-002` (explicit, keyed, bounded, invalidatable
  caches that never become the source of truth) governs `src/lib/metadata/cache.ts`.
  `ENG-INT-003` (typed errors, idempotent retries, explicit degraded results) governs
  `src/lib/providers/registry.ts`. `ENG-INT-004` (observe seam latency and outcome without
  recording secrets or sensitive payloads) binds in its client-only reading and reinforces
  rule 3 — never log titles or filenames. Only
  [`ENG-INT-005` (Credential proxy isolation)](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/integration-boundaries.md)
  (third-party credentials behind a server-side proxy) is genuinely unsatisfiable, by
  construction, which is exactly why a feature needing a private API key is out of scope.
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

**State the principle's name in a link citation, so the claim is machine-checkable.** Write
``[`ENG-INT-001` (Thin typed adapters)](…/principles/platforms/integration-boundaries.md)`` —
upstream `scripts/check-citations.mjs` then verifies the stated name against
`principles/index.json` and fails with a `claimed:`/`actual:` diff on a mismatch. This catches the
one failure mode the ID and link-path checks both miss: a real, correctly formatted, correctly
linked ID standing for a *different* rule. Only a parenthesised phrase beginning with a **capital**
is read as a claimed name, so libro's existing lowercase glosses — `(separate delivery and runtime
budgets)`, `(smallest explicit boundary…)` — are left alone; if you add a descriptive gloss, start
it lowercase or it will be checked as a name. The name is a label, not the obligation: it does not
substitute for reading the principle's **Statement** before asserting scope.

`jrmoulckers/engineering` is consumed in three layers. The first two are read from the repository
and move with its tags; the third is configuration, delivered over two channels — vendored at a
pinned ref, or npm packages whose versions track **independently** of those tags:

| Layer | What it gives libro | How it arrives |
| --- | --- | --- |
| [`principles/`](https://github.com/jrmoulckers/engineering/blob/main/principles/README.md) | 66 ratified `ENG-*` rules | cited by ID; resolve via [`principles/index.json`](https://github.com/jrmoulckers/engineering/blob/main/principles/index.json) |
| [`practices/`](https://github.com/jrmoulckers/engineering/blob/main/practices/README.md) | technique for satisfying them | linked by URL |
| `packages/` | executable enforcement | `tsconfig` + `prettier-config` vendored into `config/engineering/`; `eslint-config` from GitHub Packages — see [Shared engineering configuration](#shared-engineering-configuration) |

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
