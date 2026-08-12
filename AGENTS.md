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
| `@jrmoulckers/eslint-config` | GitHub Packages registry | **no — pending, billing only** |

**"Vendored" here describes how libro obtains a config, not whether upstream publishes it.** All
three *are* published to GitHub Packages; libro simply does not install two of them from there,
copying the source files from the public repo instead. Upstream's own metadata conflated these and
was retracted — see the note under [The vendored half](#the-vendored-half). Do not read this column
as a claim about the registry.

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

**That rule is about recipes, and it inverts for evidence.** A measurement names the artifact it was
taken from, so "verified at `v0.115.0`" is a claim about specific bytes; swapping in a resolver
re-points it at different code on every read and quietly destroys the evidence. Nearly every
measured figure in this file is deliberately pinned for that reason — the 302-line script, the
`v0.15.3` 404, the 98-tag span. Resolve a ref when you are about to *act*; pin it when you are
*reporting*.

**Do not use `gh api …/tags` to decide whether a tag exists.** It paginates at 30 and returns a
valid `200` with no indication that anything is missing, and page 1 holds only the newest tags — so
every tag it cannot see is old, which is exactly the population a stale-pin audit examines.
Reproduced here: that call returns **30** while the repository has **156** tags, an 80% false
negative rate. A false "this ref does not exist" is worse than no finding, because you act on it by
rewriting a pin that was correct. Use a form that cannot silently truncate:

```bash
git ls-remote --tags https://github.com/jrmoulckers/engineering   # no token, no pagination
gh api --paginate repos/jrmoulckers/engineering/tags --jq '.[].name'
```

libro was never exposed — its script reads `releases/latest`, not `/tags` — and all seven refs this
file cites (`v0.15.3`, `v0.18.0`, `v0.103.0`, `v0.112.0`, `v0.115.0`, `v0.116.0`, `v0.118.0`) were
re-confirmed to exist by `ls-remote`.

`pnpm vendor:check` verifies the tree against the lock. The two severities are deliberate and
worth preserving: **drift exits non-zero** (a generated file was edited or a write was lost — a
local integrity failure), while **a newer upstream release only warns, exit 0**. Failing on
staleness would make pinning automatic in effect, because a red build pressures the next person
into bumping the ref without deciding to accept the change. It runs as the first step of
`pnpm lint`, so drift is caught by CI rather than by memory.

**A green `--check` means the tree matches the lock, not that the pin is current** — and those were
two unrelated mechanisms behind one flag. The hash comparison is offline and authoritative; the
staleness notice is an unauthenticated call to `api.github.com` on every lint. So the gate
(`pnpm vendor:check`) now passes `--no-remote` and is hermetic, and `pnpm vendor:staleness` is the
deliberate online probe. Two reasons the gate is the wrong place for the network call: libro has
already been bitten by that endpoint's rate limit, which is what made an earlier verification a dead
control; and the notice keys on the repository tag while the payload has not moved in 107 tags, so
it has never once been right about what it names.

**Both no-answer cases now say so, because silence was doing double duty.** "No newer release",
"the API was unreachable", and "rate-limited" all printed nothing, so the reassuring output was
indistinguishable from a run that compared nothing. `latestRef()` returns a discriminated result and
each branch states its own gap — `--no-remote` prints *"This says nothing about whether `<ref>` is
current"*, and a failed call prints the reason plus a note that the hash comparison is unaffected.
Verified across four arms; the unreachable arm needs `NODE_USE_ENV_PROXY=1`, since **Node's `fetch`
ignores `HTTPS_PROXY` by default** and the first attempt at it printed the success path — a dead
control in the probe for the fix to dead silence.

**A guard that has never been seen failing is a claim about intent, not about the repo — so
tamper-test it.** Append a byte to a vendored file, confirm `--check` exits **1** naming that file,
then `git checkout --` it and confirm exit **0**. Measured here on `config/engineering/tsconfig/
base.json`: 1 then 0. Read the exit code from a bare invocation — piping through
`Select-Object -First` silently reports `0` for a run that exited `1`, which is the failure mode
this test exists to detect, arriving through the harness instead of the code.

**A hash comparison can only see files the lock already names, so the manifest that decides that
set has to be compared too.** `--check` walked the lock and compared each entry against disk — a
closed loop: `SETS` in `scripts/vendor-configs.mjs` is hand-edited, so adding a file there and not
re-vendoring left it absent from the lock, absent from disk, and invisible to every comparison.
Measured before the fix: `--check` exited **0** while the manifest named a file the tree did not
have. That is the fleet finding *"the lock records the ref requested, not the vintage of the tool
that wrote it"* in its libro-shaped form — the lock is evidence about a file set, and nothing was
checking it still described the set the manifest asks for.

`--check` now compares both directions and fails naming the cause: `named by SETS, absent from the
lock` and `in the lock, no longer named by SETS`. Four arms measured, so both outcomes are reachable
for the right reason — unchanged manifest **0**, a file added **1**, a file removed **1**, restored
**0**. Note the subset case is deliberately caught too: the lock is *replaced* rather than merged,
so a `--set tsconfig` run leaves the prettier files genuinely unverified, and that now says so.

Building it exposed the same class one level down. The first draft derived the expected paths with
`basename(set.from)`, which yields `prettier-config`, while the vendor step writes `join(dest, name,
file)` — `prettier`. Every entry would have mismatched, so the guard would have failed loudly on a
correct tree and been "fixed" by deleting it. **Derive a check's expectation from the code that
does the work, not from a plausible reconstruction of it.**

**Never pass `--dest` at a path outside the repo unless you mean it as a throwaway probe.** The
evaluation recipe
`vendor-configs.mjs <ref> --dest "$(mktemp -d)"` used to rewrite the real lock with **absolute**
scratch paths. `--check` then passes while examining no file in the repository at all — disarmed
rather than weakened, because every absolute path still resolves on the machine that wrote it, and
it surfaces only on a runner as `missing` on a path that never existed there. libro's lock was
never poisoned (no absolute keys, and no `--dest` run in its history), but **libro owns its own
copy of the script and carried the defect latently**, so it is fixed here rather than waited on:

- a `--dest` run **pointing outside `config/engineering`** now writes **no lock at all** and prints
  that nothing is committable;
- `--check` refuses a lock with absolute keys, naming the cause and the re-vendor command;
- the "changed content" count is keyed by **upstream source path**, not by destination. Keyed by
  destination it missed on every entry the moment `--dest` moved, reporting *all* files changed —
  meaningless in the one place the recipe tells you to read the number. Verified dest-independent:
  the same probe across `v0.115.0 → v0.18.0` reports `0 file(s) changed content`.

**That first fix was initially keyed on the wrong thing, and upstream's own refresh recipe is what
exposed it.** It tested whether `--dest` was *passed*, not where the files *landed* — so
`--dest config/engineering`, which upstream documents as the way to re-vendor, was classified as a
throwaway evaluation: the eight real files were rewritten in place while the lock kept the old ref.
Here that was harmless only because the payload was byte-identical; with a moved payload it silently
produces the inverse of the poisoned lock — correct lock, **wrong tree** — and `--check` reports the
drift as a local edit to files nobody touched. It now compares `resolve(dest)` against the default
destination, so the discriminator is the destination rather than the flag. Both arms measured:
a scratch dest writes no lock, and `--dest config/engineering` writes one.

The reusable point is that **a guard keyed on how you were invoked rather than on what you did will
misfire on any equivalent invocation** — and the invocation that tripped it was not exotic, it was
the upstream-documented one. The same reasoning retires the previous heading, which said never to
pass `--dest` at all.

```bash
node scripts/vendor-configs.mjs --check                # refuses an absolute-keyed lock
git grep -nE '"([A-Za-z]:/|/)' engineering-configs.lock.json   # any hit = poisoned
```

**`releases/latest` is ordered by tag date, not by version**, so a backported patch on an older
line can be "latest" while being *older* than the pin. The notice compared `latest !== pinned`,
which advertises that as an update — and for a lock file an update in the wrong direction is the
silent failure named under [Check the direction](#things-that-stay-true-across-both-channels):
the older payload was correct when it was current, so nothing goes red. It now compares parsed
version tuples and warns only when `latest` is genuinely newer.

Note the ordering is not merely a date-vs-version question here: libro's own tags make a **lexical**
compare wrong too, since `v0.115.0` sorts below `v0.18.0` as text — and `v0.15.4` looks larger than
`v0.115.0` character by character, the shape that has put three sibling repos on a `v0.15.x`
pin. Use numeric comparison, or `sort -V` in a shell.

**Do not replace `scripts/vendor-configs.mjs` with upstream's copy, and do not adopt the lock's
`tool` entry.** Upstream added a `tool` key that hashes the vendoring script so `--check` catches a
consumer running a stale copy. That mechanism assumes the script *is* a copy. libro's is not — it
was authored here (`758bb4a`), is not in the payload, and is not hash-locked. Two concrete costs to
switching:

- upstream's prettier set is `['index.js', 'index.d.ts', 'svelte.js', 'svelte.d.ts']`, so vendoring
  its script silently reverses libro's deliberate exclusion of the declarations — see the reasoning
  above; the `.js` already carries `@type {import('prettier').Config}` and a sibling `.d.ts`
  outranks it;
- a `tool` hash would assert that libro's script *should* equal upstream's, so every run would
  report drift for a difference that is a decision.

Upstream now ships that refresh as a literal `curl -fsSL … -o scripts/vendor-configs.mjs`, so the
instruction is no longer merely implied — running it as given silently reverts the `.d.ts`
exclusion **and** the four local fixes (`compareRefs`, absolute-key refusal, source-keyed count,
destination-keyed evaluation mode). Port a behaviour if one is genuinely missing; never take the
file.

Two of upstream's own corrections are worth recording, because they change how much weight a
broadcast claim carries. The `tool` entry was reported here as shipped when it existed **only in
their working tree**: measured, the script at `v0.115.0` is 302 lines with no `toolEntry`, and the
feature first appears at `v0.116.0` (872 lines). The cause was systemic — `publish.yml` triggers on
a manually pushed tag, so `releases/latest` sat **30 commits behind `main`** and every "this
shipped" sent to the fleet was false. That retroactively explains several rounds where a claimed
fix was unreachable from libro's ref. The rule it confirms is the one libro was already applying:
**check a claim against the consumer's ref, never against `main`** — and note that a repo can
publish a staleness notice at its consumers while its own release is stale.

**The mirror of that rule is to read the consumer's *current* ref, not a ref it once held.** libro
was later diagnosed as pinned at `v0.16.0`, 119 releases behind, with a crawl path
(`v0.15.3 → v0.15.5 → v0.15.6 → v0.16.0`) attached. That is not another repo's pin — libro really
did hold `v0.16.0`, for part of one afternoon on 2026-08-11, four pins before `v0.18.0`. So this
failure is not misattribution but **staleness about the right repo**, which is harder to dismiss
because every particular checks out against some commit in the history. It also inverts the
conclusion drawn from it: the crawl was said to hide a large aggregate by never comparing against
latest, when libro's very next move was a single **97-tag jump** (`v0.18.0 → v0.115.0`) — the
recommended remedy, already taken, and it reported `0 file(s) changed content` then too.

The cheap defence is that a pin is a **committed one-line fact**, so a diagnosis about it should
quote the line and the commit that set it:

```bash
node -e "console.log(require('./engineering-configs.lock.json').ref)"
git log --format='%h %ad' --date=short -- engineering-configs.lock.json
```

libro's **8** files and upstream's **10** are therefore two independent choices, not a version skew.
Re-confirmed by measurement, since this has been misread as a sub-floor signature three times, most
recently as a shipped diagnostic — *"if your script reported 8, you are sub-floor."* **libro is a
standing counterexample.** At libro's pin, all four upstream prettier payload files
return **200** (`index.js`, `index.d.ts`, `svelte.js`, `svelte.d.ts`) — re-measured at both
`v0.115.0` and `v0.116.0` — and a vendor run at that same
ref still writes exactly **8** — 6 tsconfig plus 2 prettier. Below the floor `index.d.ts` is a
genuine 404 (`v0.15.3`), so the count and the cause coincide *for a consumer running upstream's
manifest*, and only for them. Availability and selection are different questions; the file count
answers the second. Read the `ref` out of `engineering-configs.lock.json` to learn the pin — that is
the fact being asked about, and it cannot be inferred from a total. Upstream has since asked twice
for the *resolved version* of any consumer "still seeing no `.d.ts`" — libro sees them, takes
neither, and the answer does not change with the ref.

**libro needs no `{"type": "module"}` marker in the vendored tree, and adding one would guard
nothing.** Upstream's `prettier-config` declares `"type": "module"` and its vendoring script did not
copy that declaration, so a consumer whose root `package.json` omits `type` gets nominally-CommonJS
files whose `export default` is a syntax error — masked on Node ≥22.7, which retries the parse as
ESM. libro's root `package.json` **already declares `"type": "module"`**, and the whole repo depends
on that (`eslint.config.js`, `prettier.config.js`, `vite.config.ts`), so removing it would break far
more than the vendored configs. A marker file would restate a fact the root already states, in a
directory that must not be hand-edited.

The generalizable half is worth keeping even though the defect is not libro's: **a hash answers
"are these the right bytes", never "is this loadable."** Every file was byte-identical and the lock
verified clean while the package would not load. libro's covering gate is that the config is
**executed by something that already runs** — `prettier.config.js` re-exports the vendored
`prettier/svelte.js`, so `pnpm format:check` fails if the chain cannot load, and `pnpm typecheck`
compiles it. Verified by resolution rather than by assumption: importing `prettier.config.js`
returns a populated object, and `prettier --find-config-path` names `prettier.config.js` for both
`README.md` and `src/App.svelte` — one config source, no `.prettierrc.json` and no `prettier` field
in `package.json` to outrank it.

That is also why the vendored Prettier config's `overrides` block is in force here rather than
merely present: it sets `proseWrap: preserve` and **`printWidth: 96` for `*.md`**, which a
top-level key comparison would score as no change. `AGENTS.md` is Prettier-ignored and hand-wrapped
at 100, so the two conventions do not collide.

**Convergent fixes will not share a name — grep for behaviour, not identifiers.** libro and upstream
independently fixed the `releases/latest` ordering; libro's helper is `compareRefs`, upstream's is
`isNewerRef`. A name search reported the fix absent upstream, which would have been a confident,
checkable, wrong claim. Same class as *"the file that changed is not the file that decides"*: match
on the comparison being ordered, not on what the function is called.

**That fix was verified against a dead control first, which is why it is worth stating how.**
Driving the notice by editing `lock.ref` produced *silence in both arms* — not because the fix
worked, but because the anonymous releases API was rate-limited (`403 API rate limit exceeded`), so
`latestRef()` returned `null` and the notice could not have fired for any pin. `latestRef` is
deliberately null-safe, so **an offline, rate-limited, or unauthenticated run makes the staleness
notice silent** — its absence is never evidence that a pin is current. The ordering was therefore
tested by extracting `compareRefs` from the shipped file and running a table against it — six
cases, including the backport and the lexical trap — rather than by observing the notice.

**The staleness notice keys on the repository tag, and the vendored payload mostly does not.** Every
refresh from `v0.15.1` through `v0.122.0` reported `0 file(s) changed content`, because the upstream
tags move for docs and tooling while `tsconfig` sat at package version `0.4.0` and `prettier-config`
at `0.3.0`. Measured end to end rather than step by step: vendoring libro's manifest at `v0.15.1` —
its **first ever** pin — and again at `v0.122.0`, then diffing the two trees file by file, gives
**0 of 8 files changed across 107 tags**. All three package versions are unchanged over that span.
Each move cost a two-line lock diff and nothing else. The `v0.115.0 →
v0.116.0` step is the sharpest instance: upstream announced that release as carrying a
`prettier-config` **exports fix shipped without a version bump**, which is exactly the shape that
should move a payload — and libro's eight files were still byte-identical, confirmed both by the
tool's own source-keyed count and by an independent `Get-FileHash` of each file against the
committed tree. An upstream change is only *your* change if it touches a file your manifest names.
So through `v0.122.0` the notice had
never once been right about what it names.

**That streak ended at `v0.143.0`, and the exception is as informative as the rule.** The
`v0.122.0 → v0.143.0` refresh reports **1 file(s) changed content** — libro's first non-zero payload
move in 128 tags. The file is `tsconfig/vite-react.json`, losing `esModuleInterop`; `tsconfig` went
`0.4.0 → 0.5.0` and `prettier-config` `0.3.0 → 0.5.0` over the same span. libro vendors that file
because the manifest takes the whole `tsconfig` set, and **extends it nowhere** — verified by
`git grep vite-react` outside the vendored tree and the lock. So the payload genuinely moved, the
notice was genuinely right, and libro's effective delta is still zero. Keep all three facts
separate: *the tag moved*, *the payload moved*, and *a file you extend moved* are three different
questions, and only the third can change behaviour here.

That is also why the vendored set is deliberately not trimmed to the files libro extends. A file
libro ignores costs one hash and makes the diff report real upstream movement; pruning it would
buy nothing and hide the signal. To answer *"has my config actually changed"*, compare
the package version rather than the tag:

```bash
gh api repos/jrmoulckers/engineering/contents/versions.json --jq '.content' | base64 -d
git show <pinned-ref>:packages/tsconfig/package.json   # in a checkout of the engineering repo
```

**Both commands above are deliberately stale-proof, and the distinction is worth keeping.** A read
of a *branch* from a local checkout — `git show origin/main:versions.json` — returns whatever was
last fetched, with no error and no way to tell a stale answer from a fresh one; a repo that was
sixteen releases behind reported four facts that were all true at its ref and all false on `main`.
The `gh api` form holds no state, so it cannot have that failure. The `git show` form is safe for
the opposite reason: it names an **immutable tag**, so a cached read is by definition correct, and
a tag that was never fetched fails *loudly* with a bad-revision error rather than answering
quietly. Cache staleness is a property of reading a moving ref, not of reading locally — so never
substitute `origin/main` into either line, and if you must, `git fetch origin` is part of the
command rather than something you did earlier.

Upstream's `versions.json` records the published version per package and is registry-verified in
their CI, which makes it safe to read at a tag — unlike `packages/<name>/package.json`, which
reports the source tree at that moment and is what has repeatedly caused repos to report
already-fixed defects. Note that for a **vendored** package its `range` field is not actionable:
there is no dependency to pin, only a ref, so read `version` and ignore `range`.

**Acting on that `range` is not a harmless extra step — it undoes ADR-0001.** `versions.json`
populates `range` for all three packages while also marking two of them `channel: vendored`, so
the file simultaneously says *"this does not come from the registry"* and offers a range whose only
use is a registry specifier. Adding `@jrmoulckers/tsconfig` or `@jrmoulckers/prettier-config` to
`devDependencies` puts a private-package fetch back in the install path for every contributor and
for CI — reintroducing precisely the token requirement the vendored channel exists to remove, and
converting the one blocker that affects a single package into one that fails `pnpm install
--frozen-lockfile` outright. Read `channel` before `range`, and treat `range` as meaningful only
where `channel` is `registry`.

**Upstream's `check-pins.mjs` inherits that defect, and libro's own reading of it is the trap.**
The script fetches `versions.json` over plain HTTPS and compares each declared `@jrmoulckers/*`
range against the published version — genuinely token-free, verified here with `GH_TOKEN`,
`GITHUB_TOKEN`, `NODE_AUTH_TOKEN` and `NPM_TOKEN` all emptied, and with a positive control that
correctly flagged a synthetic `^0.8.0` as unable to reach `0.16.0`. It is a good tool for the
registry channel. But it reads **only** `dependencies` / `devDependencies` / `peerDependencies`, and
it never consults `channel`: given a synthetic `@jrmoulckers/tsconfig` range it reported `ok`, for a
package that should carry no range at all.

```bash
curl -fsSL https://raw.githubusercontent.com/jrmoulckers/engineering/main/scripts/check-pins.mjs \
  | node - ./package.json
```

On libro that prints *"no `@jrmoulckers/*` packages declared — nothing to check"* and exits **0**,
which is correct and is also the same exit code as *"every pin is current"*. **An absence answers
here with a pass**, so wiring this into CI as a pin gate would give a vendoring repo a permanent
green from a check that inspected nothing. Worse is the obvious way to "fix" a nothing-to-check
result: adding the three packages to `devDependencies` is exactly the ADR-0001 violation above.

libro *does* have a pin — the ref in `engineering-configs.lock.json` — and this tool cannot see it.
`pnpm vendor:check` is the gate that covers it, and the two are not substitutes: one checks a
**range against a published version**, the other checks **bytes against a hash**. Run both only if
libro ever adopts the registry half.

Upstream ships a top-level `channels` legend answering this as data rather than prose, so the
question can be resolved mechanically instead of by reading a paragraph:

```bash
gh api -X GET repos/jrmoulckers/engineering/contents/versions.json -f ref=main \
  -H 'Accept: application/vnd.github.raw' > /tmp/versions.json
node -e '
const j = JSON.parse(require("fs").readFileSync("/tmp/versions.json"));
for (const [n, p] of Object.entries(j.packages))
  console.log(n, p.channel, j.channels[p.channel].requiresRegistryAuth);
'
```

**As of 2026-08-11 that legend lists exactly one channel, `registry`, and all three packages use
it** — the `vendored` channel and its `requiresRegistryAuth: false` were removed outright, not
corrected in place. So the mechanical check now returns `true` three times and can no longer
distinguish the packages at all. Keep running it, but read it as *"every package requires a
token"*, which is the current fact, rather than as the discriminator it was introduced to be.

Historical note, kept because the reasoning recurs: the legend originally reported `tsconfig` and
`prettier-config` as `vendored` / `requiresRegistryAuth: false`, and that was **retracted upstream
on 2026-08-11 (v0.56.0)**. `publish.yml` publishes every directory under `packages/`
unconditionally and never reads `channel`, all three are `private: false`, and a consumer's CI got
`403 permission_denied: read_package` for `@jrmoulckers/tsconfig`. Do not resurrect the field from
an older ref, and do not draw the conclusion the retraction draws either — see below. Note that
that consumer's `403` is no longer decisive on its own: as measured under
[The registry half](#the-registry-half), a missing `read:packages` scope returns `403` too, with a
different body.

**That does not touch libro, because libro does not consume those two as packages at all.** The
retraction and the `403` are both about a *registry tarball read* on `npm.pkg.github.com`.
`scripts/vendor-configs.mjs` fetches source files from `raw.githubusercontent.com` at a tag, over
plain `fetch()` with no `Authorization` header, from a repository that is public. Different host,
different resource, different auth system — a package's registry visibility places no constraint
on reading the same bytes from the repo it is built from.

Verified rather than argued, on 2026-08-11: with `GH_TOKEN`, `GITHUB_TOKEN`, `NODE_AUTH_TOKEN`, and
`NPM_TOKEN` all emptied, `--check` passed and a full re-vendor at the pinned ref refetched all
**8/8** payload files byte-identically — the only diff was the lock's `fetchedAt` timestamp. Both
commands reach the network unauthenticated (`--check` also resolves the newest release), so either
one disproves a "blocked" claim; use the second when the question is specifically about the payload:

```bash
node scripts/vendor-configs.mjs --check     # local bytes vs lock, plus the staleness notice
node scripts/vendor-configs.mjs v0.118.0    # refetch the pinned ref; expect a fetchedAt-only diff
```

Note the second is **not** idempotent in the lock file: `fetchedAt` always moves, so a same-ref
refresh reports a change even when no payload byte differs. Read the diff before believing it —
`git diff --stat` naming only `engineering-configs.lock.json` is the no-op signature.

**`pnpm install` is token-free here too — but prove it by which host was contacted, not by clearing
credentials.** A cold run (`--frozen-lockfile --ignore-scripts`, fresh `--store-dir` *and* fresh
`--config.cacheDir`) exits 0 in 21 s and writes a genuinely populated store — 13,199 files,
172.9 MB, `reused 0` — so the packages really were downloaded rather than served from a warm cache.
The load-bearing evidence is the cache's own layout: pnpm creates one directory per registry host
under `<cacheDir>/v11/metadata/`, and libro's run creates **only `registry.npmjs.org`**. It never
asks `npm.pkg.github.com` anything, so no credential can be load-bearing regardless of what is
configured. The control run below creates that second directory, so the discriminator fires both
ways.

```bash
pnpm install --frozen-lockfile --ignore-scripts \
  --store-dir "$(mktemp -d)" --config.cacheDir="$(mktemp -d)"
ls "$THAT_CACHE/v11/metadata"       # expect registry.npmjs.org alone
```

**Redirect both, because pnpm has two caches and `--store-dir` moves only one.** The metadata cache
lives at `%LOCALAPPDATA%\pnpm-cache` (Windows) — a sibling of the store, not inside it, and
`pnpm config get cacheDir` reports `undefined` rather than naming it. A store-only "cold" probe
still resolves every packument from disk, which is the warm-cache trap one layer up from where it
is usually described.

**The credential-absence method that this replaces is unsound, and finding out cost a control run
that fired in the wrong direction.** Installing `@jrmoulckers/eslint-config` with a scoped `.npmrc`,
both caches cold, and `GH_TOKEN` / `GITHUB_TOKEN` / `NODE_AUTH_TOKEN` / `NPM_TOKEN` all emptied
**succeeded**, while anonymous `curl` against the same packument returned `401`. The reconciliation
is that **pnpm 11 keeps registry auth in its own `auth.ini`**, at
`%LOCALAPPDATA%\pnpm\config\auth.ini` — not in `~/.npmrc`, not in the project, and **not listed by
`npm config list`**, which is what the earlier hunt used and why it reported no credential anywhere.
So an env-var sweep plus an npm-config read can both come back clean on a machine that is fully
authenticated.

Two transferable points. A probe that clears every credential it *knows about* measures the
searcher's inventory, not the machine — prefer evidence of what was **contacted**, which does not
depend on enumerating auth sources correctly. And this is the fourth instance of the same shape: the
control was the only thing in the run that could have failed, and it failed for a reason nobody had
proposed. **A control firing unexpectedly is the finding, not noise to re-run.**

The generalizable point, which is why this is written down rather than deleted: **"is it published"
and "can I obtain these bytes" are different questions, and only the second one is libro's.** The
upstream claim, its retraction, and the `403` evidence all answer the first. A retraction inherits
the scope of the claim it withdraws, so it can be entirely correct and still not reach you — check
which question your own mechanism asks before acting on either.

The second command above is the same file those repos misread, and it is correct here only
because it answers a different question. `versions.json` answers *"what is published"*;
`packages/<name>/package.json` at a ref answers *"what was the source tree at that ref"* — which
is precisely the vendored-payload question, since the payload is fetched from the ref. Use it to
compare two refs you are choosing between. Never quote it as a published version.

**If `versions.json` is absent from the ref you are reading, that absence is the finding.** It
first appears at `v0.18.0`, which was libro's pinned ref for most of the migration, so libro sat on
the boundary: any ref below it has no `versions.json` at all, and the only version-shaped file in
the tree is the `package.json` that must not be quoted. This compounds with the direction rule
above — a backwards re-pin does not merely revert workflow content, it can remove the file that
exists to prevent the version error, leaving the misleading answer as the only available one.
Resolve a newer ref before reporting anything about versions.

**libro vendors 8 files because its manifest says 8, not because the ref is old — and the number
coincides with a real sub-floor signature.** Upstream reports a full set of 10 and reads `8` as
*"this repo predates the two `prettier-config` declarations."* That inference is sound upstream and
false here: `SETS` in `scripts/vendor-configs.mjs` hardcodes `files: ['index.js', 'svelte.js']` for
prettier, so a refresh at **any** ref fetches exactly those two, and re-pinning 107 tags forward
changed the count not at all. The count is a property of the consumer's manifest, and a diagnostic
keyed on it silently assumes every consumer takes the whole set.

**Do not add `index.d.ts` / `svelte.d.ts` to that manifest.** They exist upstream from `v0.112.0`
and are genuinely needed by a *package* consumer, whose `.js` is opaque to the compiler. libro is a
**source** consumer, and the two facts that follow both cut the other way: the vendored
`prettier/svelte.js` already carries `@type {import('prettier').Config}`, so `prettier.config.js`
resolves to `Config` today — verified by probing a known key, which errors `Type 'Config' is not
assignable to type 'number'` — and a sibling `.d.ts` **outranks the `.js` it sits beside**, so
vendoring one would replace a type read from the code libro actually runs with an assertion about
it. That is the ambient-shim precedence hazard in a second form: same mechanism, and here it would
be self-inflicted while adding nothing. For a source consumer a declaration is strictly weaker than
inference — it can only introduce a way for the type and the executed code to disagree.

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
- **`esModuleInterop`** — dropped on the app project and **not** re-added, and the reason recorded
  here was wrong in libro's favour. It said `svelte-check` is clean because nothing in `src/`
  default-imports a CommonJS module — which is the *green gate hides the delta* mistake this file
  warns about: it cannot distinguish "the option was not needed" from "the option was lost and
  nothing depends on it yet." Measured instead, with a live control. Against libro's own vendored
  `base.json` a default import of an `export =` module compiles **clean**; add
  `allowSyntheticDefaultImports: false` and the same file fails **TS1259** naming that flag. So the
  pass is caused by `moduleResolution: "bundler"` implying the flag, not by an absence of demand,
  and `noEmit: true` makes the emit half moot. The same two arms run against `vite-node.json` give
  the same result, which means the node project's local `allowSyntheticDefaultImports: true` is
  **inert today** — it was previously described here as the one genuine local retention in either
  file, and that is not true. It is kept as an unenforced guard: it would become load-bearing the
  moment any project overrides `moduleResolution` away from `bundler`. Nothing in libro tests that,
  so treat it as a comment with an effect rather than as a control.
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

**`include` without `allowJs` is a silent no-op, and libro had it — three files listed and never
compiled.** `tsconfig.node.json` names `vite.config.ts`, `svelte.config.js`, `eslint.config.js`, and
`prettier.config.js`, but the vendored `vite-node.json` sets no `allowJs`, so only the `.ts` file
entered the program. There is no error and no warning: `tsc` exits 0, and the `include` entry makes
it *look* as though the other three are checked. Measured rather than inferred — `--listFilesOnly`
returned exactly one non-`node_modules` path. `allowJs: true` plus `checkJs: true` is now set
locally on that project; all four compile and the check is still clean.

Read the second-order effect, because it is what makes this worth writing down: a config file that
is silently outside every project **cannot report a type error at all**, so adopting typed presets
there yields nothing and reads as *"the declarations don't work"* rather than *"the file isn't
compiled."* Verify with `npx tsc -p <project> --listFilesOnly`, never with the exit code — the
failure mode is exit 0 by construction. Note also the intermediate arm: `allowJs` alone puts the
files in the program but reports no errors in them, so the file list and the error count answer
different questions and you need both.

One coupling this buys, stated so it is not a surprise later: `prettier.config.js` re-exports the
vendored `config/engineering/prettier/*.js`, which now enter the program too. That is mostly a
feature — a broken vendored payload becomes a failed typecheck — but it means an upstream refresh
can turn this gate red on files libro must not edit. Fix such a failure upstream or by re-pinning,
never by editing the vendored file.

### The registry half

`eslint.config.js` is still authored locally, but **not because the package is ungrantable** — an
earlier revision of this section said so and was wrong. All three packages report
`visibility: private` (`gh api /user/packages/npm/<name>`), yet a package **linked** to a repository
inherits that repository's access permissions, and `jrmoulckers/engineering` is public
(`"private": false`). So the `private` label describes the package record, not the grant, and no
per-package "Manage Actions access" flip is required. Upstream retracted the blocker after two
consumers installed it in CI on `GITHUB_TOKEN` alone with `packages: read`.

**libro cannot verify that claim, and the reason is worth more than the claim.** Every credential
available here belongs to the account that owns the packages, so a `200` is equally consistent with
"public inheritance" and "the owner can always read their own private package." The probe cannot
distinguish the hypotheses, which makes a passing result no evidence at all — the same defect as a
control run that cannot fail. Only a token belonging to someone who is *not* the owner settles it,
which is exactly what the consumer CI logs are. Accept it on their evidence, not on ours.

**What libro's own probe does settle is the 401/403 reading, and it corrects this file.** Three
arms against `https://npm.pkg.github.com`, same package, same moment:

| credential | packument | body |
| --- | --- | --- |
| none | `401` | `authentication token not provided` |
| PAT **without** `read:packages` | `403` | `permission_denied: The token provided does not match expected scopes` |
| PAT **with** `read:packages` | `200` | tarball also `200`, 13,945 bytes |

This file used to say that `403 permission_denied` is authorization and *"no amount of token work
resolves it."* That is false: the middle arm becomes the bottom arm by changing nothing but the
token's scopes. **There are two different 403s, and only the body distinguishes them** — a scope
mismatch, which you can fix, versus a genuine package-access denial, which you cannot. Read the
response body, not the status code; a rule keyed on `403` alone sends you to an owner-only setting
for a failure that is yours to fix. Note also that the earlier "metadata resolves, only the tarball
fails" tell never fired here — the scope failure blocks the packument too, so absence of that
signature does not mean the wiring is fine.

**Nothing about the registry half is a libro blocker any more**, so the remaining wait is billing
alone. The practical consequence is unchanged: **a lockfile may be generated locally but must not
be committed until CI can install**, because a lockfile CI cannot resolve fails every job rather
than only the lint step.

When it is unblocked, adopt at `@jrmoulckers/eslint-config@>=0.15.0 <1.0.0` and replace the local
file with `svelteConfig()`; libro's current rule set is a strict subset, so nothing is lost.
Latest read from the registry on 2026-08-12: `eslint-config` **0.17.0** (18 published versions),
`prettier-config` **0.5.0**, `tsconfig` **0.5.0** — the latter two are vendored here, so they have
no specifier and their floors are informational only.

**Read `dist-tags.latest` from a packument; never take the last key of `versions`.** GitHub Packages
serves that object **lexically sorted**, so for `eslint-config` the key order ends
`… 0.7.0 0.8.0 0.9.0` while `dist-tags.latest` is `0.17.0` — the last key is eight minors stale and
looks like a clean answer. `time` has the same order, so a "most recently published" read off it
fails identically. The trap needs ten releases to appear: `tsconfig` and `prettier-config` have five
each, where lexical and numeric order coincide and the last key is correct. So a script doing this
is right on two packages out of three, which is the worst possible amount of right.

```bash
npm view @jrmoulckers/eslint-config version          # dist-tags.latest
node -e "…; const v=Object.keys(p.versions).sort(cmp).pop()"   # only with a numeric cmp
```

This is the lexical-comparison hazard recorded above for repo tags (`v0.115.0` sorting below
`v0.18.0`), arriving through a JSON object's key order rather than through a string compare anyone
wrote. Nothing in the response is malformed and no call fails.

**A `>=x <1.0.0` floor names a minimum, not a target — so a floor bump needs a defect to justify
it, not a newer release.** Any version above the floor installs regardless; that is the whole point
of dropping the caret. For libro the *binding* release is still `0.14.0`, the `untypedFiles` crash
fix, and `0.15.0` is alignment rather than necessity: hashing both tarballs file by file,
`svelte.js` is **byte-identical**, as are `dependencies` and `peerDependencies`. Only `base.js`,
`ignores.js`, `package.json`, `README.md` move, plus a new `ignores.d.ts`. So if a resolution ever
lands on `0.14.0`, that is not a defect here — which is worth knowing before spending a change on
it.

**`0.15.0`'s actual content is the enumeration bug again, in the ignore direction.** `toolingFiles`
goes 9 → 24 globs and is exported as `@jrmoulckers/eslint-config/ignores`. The upstream docblock
gives the reason: the old list carried `*.test.js` but not `*.spec.js`, and `*.config.mjs` but not
`*.config.cjs`, so whether a file counted as tooling depended on which of two interchangeable
suffixes its author picked. That is the same root cause as the `strictTypeChecked` crash — a list
built by **enumerating extensions** silently omits what it does not name — reached from the other
side, and it is why the fix is *exhaustive list plus an export to extend* rather than one more
glob.

libro is not affected either way: its single tooling file, `scripts/vendor-configs.mjs`, matches
`**/scripts/**/*.mjs` at **both** versions. Spread `toolingFiles` rather than re-authoring it if
`scripts/` ever grows a `.cjs` or a `*.spec.*` file.

**`0.16.0` splits the type-checked flags — and ships them undeclared, so hold the floor at
`0.15.0`.** `base.js` gains `typeChecked` (correctness only) and `stylisticTypeChecked` (house
style), with `strictTypeChecked` retained as an alias for both. But `types.d.ts` and `base.d.ts` are
**byte-identical to `0.15.0`**, and `BaseOptions` declares only `typeAware`, `strictTypeChecked`,
and `untypedFiles` — with **no index signature**. So the two new options are implemented and
unannounced: passing either from a `checkJs` config is a hard `TS2353` naming `BaseOptions`, and the
release's headline feature is unreachable from exactly the consumer the declarations exist to serve.
That is the same implementation-versus-declaration split the release is otherwise about, and it is
why `0.16.0` is not a floor bump for libro: it fixes nothing libro has and adds a feature libro
could not call.

**Measured against libro's own `App.svelte` at `0.16.0`, not argued.** The split is real and the
runtime half works; only the declaration is missing:

| arm | result |
| --- | --- |
| `svelteConfig({ strictTypeChecked: true })` | exit 1 — 2 findings, **both stylistic** (`consistent-type-definitions`, `array-type`) |
| `svelteConfig({ typeChecked: true })` | **exit 0** — and `no-floating-promises` still `2` on a `.ts`, `consistent-type-definitions` absent |
| the same config under `checkJs` | **`TS2353`: `'typeChecked' does not exist in type 'BaseOptions'`** |

So libro's entire `strictTypeChecked` finding count is house style, `typeChecked` would buy the
correctness half for free, and libro is the exact consumer that cannot ask for it — because
`tsconfig.node.json` now compiles `eslint.config.js`. Enabling `checkJs` is what converts this from
someone else's release note into a blocker here, which is worth knowing before adopting: **a
stricter gate can turn an upstream declaration gap into your build error.** That is not a reason to
weaken the gate; it is a reason to report the gap.

**Adoption is additionally npm-hostile on ESLint 10, though not for libro.** `0.16.0` declares
`eslint-plugin-jsx-a11y ^6.10.0` as an *optional* peer, and that plugin's own peer stops at
`eslint@^9`. Measured: `pnpm add` succeeds with a peer **warning** (libro's manager, so libro is
fine), while `npm i` fails `ERESOLVE` outright — and it fails while leaving `jsx-a11y` **absent**
from the tree. That refines the optional-peer rule recorded above rather than contradicting it:
npm does not *install* an optional peer it can skip, but it does *validate* that peer's own peer
range and abort. **Not-installed and not-considered are different**, so "optional" bounds what
lands on disk, not whether the install succeeds.

**It also defeats the shim discriminator, which is the transferable part.** The published rule for a
stale hand-written `declare module` is to read the *shape* of the failing type — a **named** type
means the package's real declarations, an **anonymous type literal** means a shim. Here the error
names `BaseOptions`, so the discriminator reports *healthy* on a genuine defect. It separates
*whose* declarations you are reading, not whether they match the implementation, and only the second
question is the one a rejected-but-valid option raises. When a valid option is refused, hash the
`.d.ts` against the previous version before believing the error.

**The companion recipe — "check that a bogus option is still rejected" — fails in the other
direction, and the two failures together retire it.** It is meant to catch a declaration that
resolved but widened to `any`. Measured against libro's own toolchain, both arms in one `checkJs`
run:

| declaration | a bogus option | a valid option |
| --- | --- | --- |
| Prettier's `Config` (carries `[_: string]: unknown`) | **accepted** — probe is vacuous | fine |
| `BaseOptions` (no index signature) | rejected | **also rejected**, wrongly |

So the probe reports "broken" on a healthy declaration that is deliberately open, and "healthy" on
one silently behind its implementation. Which answer you get is decided by whether the type has an
index signature — the very property you would need to know already in order to read the result.
Probe a **known** key with a **wrong** type instead: `/** @type {number} */ const n = config.semi;`
raises `TS2322: 'boolean | undefined' is not assignable to 'number'`, and that works on both shapes
because it depends on neither. Verified here against prettier 3.9.6, whose `index.d.ts` does carry
`[_: string]: unknown`.

The reusable shape: a probe is a test only if both its outcomes are reachable **for the reason you
think**. Both recipes above answer confidently in cases they cannot actually distinguish — the same
defect as a control run that cannot fail.

libro has **no `declare module` shim to remove** — the grep returns nothing, and could not have
returned anything, since libro never took these as packages at all. Its only `.d.ts` files are
`src/vite-env.d.ts` and five sync-owned files under `vendor/@jrm/tokens/`.

That coverage claim survived a wrong measurement of mine worth recording, because it failed in the
one way a parse usually does not. Extracting the glob array textually from `0.15.0` returned
`['services/**/*.ts', 'no-console', 'off']` — the docblock's *example* array — and reported
`scripts/` as uncovered. The parse succeeded, returned a plausible-looking list, and was only
obviously wrong because two entries were **rule names rather than globs**. A result of the wrong
*kind* is detectable; a result of the right kind and wrong content would have shipped. Read the
source when a source-derived figure contradicts an upstream claim, rather than reporting the
contradiction.

**The one thing adoption still adds is the Prettier interop, and its surface here is exactly one
rule.** libro's config never applies `eslint-config-prettier`, which is the gap the migration brief
opened with — but "the formatter and the linter can fight" names a risk without bounding it, so
measure it rather than repeating it. Intersecting libro's **enabled** rules against the 178 that
`eslint-config-prettier` switches off gives **1** on both file classes: `no-unexpected-multiline`.
Every other one of those 178 is already absent or off here. So adoption is still correct, and the
behaviour change it brings is a single rule, not a category.

Read that intersection against **enabled** rules only. `--print-config` lists a rule at severity `0`
identically to one that runs, and the interop's 178 span nine namespaces — 81 core, 39 `vue/*`,
19 `@typescript-eslint/*`, 16 `react/*`, 11 `flowtype/*` — so a name-based count of "rules the
interop touches" is inflated by plugins the repo does not even load. **Grep the severity, not the
name.** libro resolves 129 rules on a `.svelte` file and 105 of them run.

**Keep `eslint-plugin-svelte` in `devDependencies`** — libro already declares it, and must
continue to, because `svelte.js` imports the plugin with a static top-level `import`. Omitting it
fails at config load naming the package, rather than degrading silently. The declared range is
`^2.46.0 || ^3.0.0`, which libro's 3.22.0 satisfies, so the 0.12.0 change below cannot bite here.

The reason that declaration is load-bearing has been restated upstream twice, and the middle
version was false. Through 0.8.0 the plugins were optional peers; from 0.9.0 their ranges moved to
a bespoke `frameworkPlugins` field that npm ignores, justified by the claim that npm 7+
auto-installs any *optional* peer it can resolve. **It does not.** Verified here on npm 11.16.0
with a two-arm probe — a throwaway provider packed with `npm pack` and installed into a bare
consumer:

| provider declares | peer installed? |
| --- | --- |
| `peerDependencies` + `peerDependenciesMeta.optional` | **no** |
| `peerDependencies` alone (control) | **yes** |

So the asymmetry is real but sits at *required vs optional*, not at *errors vs installs*, and the
original optional-peer design was never the cause of the install-size measurement it was changed to
fix. 0.12.0 restores them as real optional peers — confirmed later by reading every published
packument, which also dates the removal precisely: 7 peers through `0.8.0`, 2 at `0.9.0`–`0.11.0`,
7 again from `0.12.0`. Keep the control arm in any such probe: without
it, "not installed" is equally consistent with a probe that installed nothing at all.

The transferable rule is that **a measurement is evidence for its number, not for its cause** — the
75 MB was real throughout while the mechanism blamed for it was not. When reporting install weight,
send `npm ls <pkg>` or `pnpm why <pkg>` alongside the figure, because the dependency path is the
part a reader can check.

Upstream later reproduced this independently — packing the tarball at `0.17.0` and at `0.8.0` as a
control, and finding all five framework plugins absent in both. That is genuine corroboration,
because the two probes could have disagreed and did not. It also **retires the counter-example
offered for the caret**: if no consumer ever received these plugins from the package, then the
`0.9.0` peer removal changed nothing observable. A consumer who declared `eslint-plugin-svelte`
worked before and after; one who did not was already broken at `0.8.0`. So the removal is not a
breaking minor for anybody, and the argument that a wide range "takes a breaking minor blind" has
no instance behind it here.

**Write the range that way, not as a caret.** On a `0.x` package a caret permits patch updates
only — `^0.9.0` is `>=0.9.0 <0.10.0` and can never reach 0.10.0 — so a caret floor silently
freezes you on the minor you pinned. Confirmed rather than recalled: `semver.validRange('^0.4.1')`
is `>=0.4.1 <0.5.0`. The failure is invisible in the worst way: install succeeds,
CI stays green, and you go on reporting defects that were fixed several releases ago, because the
fix is unreachable. An explicit `>=x <1.0.0` is the only form that tracks a pre-1.0 package.

**Upstream has since reversed that advice back to `^0.17.0`, and the counter-example it rests on
cannot reach libro's range — measured.** The argument is that a `0.x` minor may break, so a wide
range takes a breaking minor blind; the evidence offered is `eslint-config`'s framework peers being
removed and later restored. Both events are real, but the enumeration is off and the conclusion does
not follow here. Reading `peerDependencies` from the registry packument for all 18 published
versions: 7 peers through `0.8.0`, **2** at `0.9.0`–`0.11.0`, back to 7 from **`0.12.0`** onward —
so the window is three releases, not the eight implied by naming `0.16.0` as the restoration.
libro's declared floor is `>=0.15.0`, which puts the **entire window below the floor**;
`semver.satisfies` confirms `>=0.15.0 <1.0.0` reaches none of `0.9.0`, `0.10.0`, `0.11.0`. A wide
range floored above a defect is not exposed to it, and the caret is not what protects you — the
floor is.

Two further reasons to keep the wide form here. Today the two ranges are **indistinguishable**:
`^0.17.0` and `>=0.17.0 <1.0.0` both resolve to `0.17.0`, so the choice only becomes observable at
`0.18.0` — it is a bet about future releases, not a fix for a present defect. And the stated
mitigation for the caret's stranding failure is that `check-pins` reports staleness; as measured
under [The registry half](#the-registry-half), that tool reports *"nothing to check"* and **exits
0** for libro, because libro declares no `@jrmoulckers/*` dependency at all. So adopting the caret
here would take the freeze without the compensating signal. Revisit if libro ever adopts the
registry half — and note libro is doubly insulated from the specific failure anyway, since it
declares `eslint-plugin-svelte` in its own `devDependencies`, so the peer's removal cannot strand
it.

**When you do move a floor, diff against the floor — not against the next version.** The rigorous
check on a bump is to print the effective resolved ruleset on both versions and diff it, and that
check is sound; what it cannot do is generalize. A diff of `0.6.0` against `0.7.0` is evidence about
those two versions and nothing else, so an empty result licenses "this bump changes nothing" and
ends the investigation, while the eight releases between there and the floor go unexamined. Compare
the version you are on against the version you are going to.

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

**0.11.0 fixes a `.svelte` rule-scoping defect that libro's local config had independently.**
`typescript-eslint`'s `eslint-recommended` layer is scoped to `**/*.{ts,tsx,mts,cts}` — read off the
layer's own `files` glob, not inferred — so it never reaches `.svelte`. Because that one layer both
disables the core rules the compiler already enforces and enables four others, components run
**19 rules wrongly on and 4 wrongly off**. `no-undef` is the one that bites: it cannot see ambient
or namespaced types, so a component
referencing `NodeJS.Timeout` errors while byte-identical `.ts` code does not — and the identifier
is correct, so it is unfixable in the source. Reproduced here with a two-file probe before the
fix, and the local config now re-applies the layer to `.svelte`.

That figure was **18** here for several revisions, because it was taken from a resolved-config diff
rather than from the layer. The two count different things and both are correct answers: a diff
reports the layer's *net* effect and silently nets out any rule some other config already had in
the same state, while enumerating the layer reports its *size*. Prefer the layer when the claim is
about what the layer contains — a diff will drift as the configs around it change, and it shrinks
precisely when another config starts duplicating the layer, which reads like the layer mattering
less rather than more.

Verify a rule-scoping claim by diffing **resolved** config per file class, not by reading the
config source — the defect is invisible there, and a green lint is not evidence, because it only
fires once a component references such a type:

```bash
npx eslint --print-config src/App.svelte
npx eslint --print-config src/lib/epub/epub.ts
```

Post-fix those two differ by exactly one rule, `no-self-assign`, which the Svelte plugin disables
deliberately. That is the check to re-run on adoption, and it is worth running against any scoped
exception to confirm it is as narrow as its comment claims. The tradeoff the layer carries — a
`.svelte` file with a plain `<script>` gives up `no-undef` too — does not bite here: all three
components are `lang="ts"` and `svelte-check` type-checks them. Reconsider if one is ever added
without it.

Where a preset defect is reported, quote the **resolved** version — but **not** with
`node -p "require('@jrmoulckers/eslint-config/package.json').version"`, which is the obvious recipe
and fails in the worst possible direction. `./package.json` entered the package's `exports` map
somewhere after `0.13.0`, so that command returns cleanly on a current version and throws
`ERR_PACKAGE_PATH_NOT_EXPORTED` on an old one — measured, `0.13.0` exits 1 with a stack while
`0.17.0` prints `0.17.0`. A version probe that only works once you are already new enough is
useless precisely when you need it, since the reason to ask is almost always that you suspect an
old pin. Worse, the throw reads as a broken install rather than as an answer. Use a route that does
not go through `exports`:

```bash
npm ls @jrmoulckers/eslint-config --depth=0
node -e "console.log(JSON.parse(require('fs').readFileSync('node_modules/@jrmoulckers/eslint-config/package.json','utf8')).version)"
```

Both return `0.13.0` on the version that throws. A pinned range is not an
observation; under a caret freeze the two routinely disagree, and that gap is exactly what makes
the freeze invisible. For the same reason, never read a package's version or peers from a *repo*
tag: `v0.4.0` of the engineering repo shipped `eslint-config` 0.3.0, so the ref resolves cleanly
and answers the wrong question.

A defect worth knowing about on adoption, now **fixed** — `strictTypeChecked: true` used to abort
the entire ESLint run on the first `.svelte` file, because the type-checked rule sets applied
unscoped while the re-disable blocks that rescue `.ts`/`.js` matched neither `.svelte` nor its
variants. It was present in **every published version that had the option** — `0.6.0` through
`0.13.0`, eight releases — and is **resolved at `0.14.0`**, which parameterises the trailing block
as an `untypedFiles` option that `svelteConfig` passes its own globs into. That is the general fix
rather than a fourth hardcoded `.svelte` block, so the next preset covering a file type no
`tsconfig` can include does not rediscover it.

The root cause is worth stating in its general form, because it is not "a fix missed a case": the
disable blocks were written by **enumerating extensions**, and an enumeration silently omits
whatever it does not name. `.svelte` was never covered rather than uncovered by a regression.
Parameterising the glob is what removes the class.

Verified end-to-end on 2026-08-11 against libro's own sources, **with a positive control**, because
"it passes now" is not evidence unless the same harness can still produce the failure:

| arm | result |
| --- | --- |
| `0.13.0`, `strictTypeChecked: true` | **exit 2** — `await-thenable`, `Parser: svelte-eslint-parser`, nothing linted |
| `0.14.0`, `strictTypeChecked: true` | **exit 1** — 2 real findings in `App.svelte` |
| `0.14.0`, bare `svelteConfig()` | exit 0 |
| `0.14.0`, `no-floating-promises` on a `.ts` | still `2` (error) — typed linting intact |

Re-measured at **`0.17.0`**, the newest published version, against the same `App.svelte` and with
`0.13.0` re-run as the control in the same harness: `0.13.0` **exit 2**, aborting on the parser;
`0.17.0` **exit 1** with the same two stylistic findings plus two more on a companion `.ts`. So the
fix holds at the head of the line, and the control still reproduces the original failure — which is
the only thing that makes the pass meaningful.

**Do not expect a vendored re-pin to change this, and do not accept a diagnosis that says it
will.** `strictTypeChecked` lives in `eslint-config`, which libro consumes over the **registry**
channel and has not adopted; the vendored `SETS` names `packages/tsconfig` and
`packages/prettier-config` only. So moving `engineering-configs.lock.json` forward cannot alter
ESLint behaviour by construction, however many releases it spans. A prescribed experiment whose
outcome is fixed in advance by which channel a package arrives on will "confirm" whatever it is
pointed at; check the manifest before running it. The same reasoning disposes of the related claim
that a re-pin would make two `.d.ts` files appear — they are in upstream's prettier set, not in
libro's, which names `['index.js', 'svelte.js']` at every ref.

Note the near-miss in that run, which is the reusable part: with the config placed beside the preset
under `node_modules/`, ESLint exited **0** while reporting *"File ignored because outside of base
path"* — a pass that measured nothing, in the exact shape this file warns about elsewhere. Read the
file count before reading the exit code.

**Hash the artifact before re-verifying it against a new version number.** Across all 15 published
versions `base.js` has exactly **three** distinct contents — `0.1.0`–`0.5.0`, then `0.6.0`–`0.13.0`
byte-identical across eight releases, then `0.14.0`. This file previously recorded the defect as
"present at `0.12.0`, re-confirmed at `0.13.0`" as though that were two observations; it was the
same bytes read twice. Repeated verification against an unchanged artifact accumulates confidence
without accumulating evidence, and it is indistinguishable from the real thing at the point of
reading. `svelte.js` moved at `0.11.0` while `base.js` did not, which is the same trap from the
other side: **the file that changed is not the file that decides.**

One behavioural change to know: the `extend` self-rescue for `.svelte` **no longer works**, and that
is deliberate. It only ever worked because no trailing block matched `.svelte`, so a caller's entry
was last-matching by accident; now an `extend` entry re-enabling `no-floating-promises` on `.svelte`
resolves to `0`. Drop the workaround rather than reinstating it.

The retired warning is kept in outline because its reasoning recurs: the earlier `.svelte` scoping
fix landed in `svelte.js` while this defect lived in `base.js`, so a release note that read "the
Svelte path was fixed" left it untouched. **A workaround for a fixed bug is just a bug — but
retiring one on the strength of a nearby fix is worse, because the note makes it feel verified.**
Retire one only against a direct two-arm measurement like the table above — and note that a probe
which does not enable `strictTypeChecked` cannot fail on *any* version, so it certifies the fix on
releases that predate the mechanism entirely.

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

**Check the *direction* of a proposed pin before its content.** A SHA arrives as an upgrade, but a
SHA broadcast to several repos sitting at different pins is necessarily a rollback for some of
them, and a backwards re-pin is the one workflow failure with **no signal at all** — the older
callees were green when they were current, so CI stays green while whatever they fixed is silently
withdrawn. Compare first, and refuse anything that reports `behind`:

```bash
gh api repos/jrmoulckers/.github/compare/<current-pin>...<proposed-sha> \
  --jq '{status,ahead_by,behind_by}'
```

Run it from libro's **actual** pin. A sender's distance figure is measured from whatever ref they
believe you are on, so it is stale the moment you move: `68c4265` was offered as 163 commits ahead,
which was true of `f145727` and wrong by 157 of the pin libro already held.

**Then compare the callees' blob SHAs, not the commit distance.** A pin exists to freeze *content*,
so "behind by commits" and "behind by content" are different questions and only the second one
matters. The five call sites were held at `f145727` through a stretch where that ref was 157 commits
behind `main` and every called workflow was still byte-identical, because
the intervening commits touched the sync engine, docs, and workflows libro does not call. Ask the
contents API for each callee's `sha` at both refs; a bare `compare` file list answers it too, but
only if you check it against the call sites rather than skimming it. Re-pinning to a ref whose
callees are identical is churn that can only introduce error.

The current pin is `974154d`, taken when `reusable-perf-budget` gained an `exclude-glob` input;
the other four callees were byte-identical to `f145727` at that point. That input defaults to `''`,
which measures the whole output directory exactly as before, and libro emits no source maps into
`dist/`, so it is deliberately left unset — set it to `'*.map'` only if a build starts emitting
them, or the budget will be spent on bytes no user downloads.

**`packages: read` is mandatory on the caller**, and it tracks what the callee *declares*, not
whether it installs. `reusable-perf-budget` is the case that proves the distinction: its two
install steps are conditional on `artifact-name == ''`, and libro passes an artifact from the `web`
job, so **libro's invocation runs no install at all** — yet the grant is still mandatory, because
the callee declares `packages: read` unconditionally at the job level, and permissions resolve
before any `if:` is evaluated. Do not restate that as "perf-budget does not install": it does, for
callers that build standalone. The install is a property of the invocation; the declaration is a
property of the callee, and only the declaration decides the grant. A caller
`permissions:` block replaces the defaults rather than adding to them, and a called workflow can
never hold a scope its caller lacks, so omitting one fails the run at `startup_failure` with no
readable log. `reusable-ci-lint` additionally needs `pull-requests: read`. `actionlint` does not
model this ceiling, so a green lint is not evidence.

**The blast radius of that failure is the whole file, not the offending job.** Permission
resolution runs before any job is created, so one caller job missing a scope its callee declares
takes down every other job in the same workflow file — including jobs that delegate elsewhere and
are individually correct. libro's `ci.yml` fans out to five reusable workflows, so the failure mode
is all five vanishing at once with no check run and no log. Nothing written inside the file can
detect it, and **a green history is not protection**: the ceiling only binds once the pin moves to
a ref whose callee declares the scope, so a run that has passed for months can go dark on a re-pin
that changed nothing else. Re-pin and re-verify every caller/callee pair **in the same commit**,
by reading each callee's declared `permissions:` at the *target* ref — not the current one.

**Do not diagnose a logless, step-less failure by elimination — count the jobs, then ask for the
annotation.** At least three distinct causes produce a fast, logless failure: the permission ceiling
above, an unresolvable pinned ref, and a **billing hold on the account**. The four probes people
reach for first — the log, `output.title`, `output.summary`, and the step count — cannot vary with
the cause, so drawing a conclusion from four empty results is reading noise.

The job count does vary, and it separates the ceiling from the other two by mechanism rather than by
correlation: a caller asking for a scope it does not hold is rejected *before any job is created*,
whereas a billing hold creates every job and then cannot allocate a runner.

| Cause | `/jobs` | `/timing` |
| --- | --- | --- |
| caller grant below callee | **0 jobs** | `billable: {}`, `run_duration_ms: null` |
| billing hold | **all jobs present**, `steps: 0` | `UBUNTU` present, `total_ms: 0` per job |

Measured on libro's run `31562081343`: 8 jobs (5 `failure`, 3 `skipped`), every one `steps: 0`,
`total_ms: 0` across 6 billable jobs, and `run_duration_ms: 12000` — a run that lasted twelve
seconds and was billed for none of it. **No jobs means it was never admitted; jobs with zero time
mean it was admitted and never ran.** Note that *absent* and *zero* billable time are different
answers; treating both as "zero billable time" is what collapses the two causes into one signature.

Ordering matters, because the annotation recipe reads `.jobs[0].id` and therefore has nothing to
index in the ceiling case — on its own it can only diagnose the cause it was first tested against:

```bash
run=$(gh api "repos/jrmoulckers/libro/actions/runs?per_page=1" --jq '.workflow_runs[0].id')
gh api "repos/jrmoulckers/libro/actions/runs/$run/jobs" --jq '.jobs | length'   # 0 => ceiling
job=$(gh api "repos/jrmoulckers/libro/actions/runs/$run/jobs" --jq '.jobs[0].id')
gh api "repos/jrmoulckers/libro/check-runs/$job/annotations" --jq '.[].message'
```

On 2026-08-11 that returned, for libro: *"The job was not started because recent account payments
have failed or your spending limit needs to be increased."* Owner-side, and it gates **all** CI on
every private repo in the studio, so a red check here is not evidence of anything in the diff. The
ceiling's prescribed fix (grant more scopes) does nothing for it, so eliminating causes by trying
their remedies leads away from the answer. Public repositories are unaffected, which makes the split
look like a configuration difference between two repos whose configuration is identical.

**A control run is only valid if it ran in the same platform state as the test.** The usual control
for *"did my change break CI"* is the changed branch against an unmodified default branch — but once
the account is held, the default branch fails too, so the comparison returns "both red" whatever the
change did. The comparison still completes and still looks like a controlled one; it has simply lost
the ability to vary. Before treating a red run as evidence about a diff, confirm the account can
produce a green **at all** — a public repo under the same account is the cheapest check, since its
minutes are not billed.

libro's own boundary, measured rather than inherited: last green `2026-08-10T21:34:11Z` on `main`
(seven checks), first red `2026-08-10T21:58:20Z`, and **that first red already carries the billing
annotation** — as do the two after it, at 8 jobs / 0 steps each. So there is no window of ordinary
failures preceding the hold to confuse it with. Take the boundary from this repo's own run list;
a fleet-wide timestamp will be close but not identical, and the interesting question is always
whether *your* last green predates *your* first red by content or by platform state.

That green is what keeps libro out of the confounded case: a repo whose entire retained run history
postdates the hold has never observed its own CI pass, so it has no baseline to compare against and
cannot tell a broken pipeline from a held account by inspection at all.

**The converse matters more while the hold lasts: a check that is already red for a known reason
has stopped being a signal for anything landed underneath it.** The rule above says a red run is
not evidence *against* the diff; this says it is not evidence *for* it either. Both conditions
surface as one status in one colour, so a second, unrelated failure introduced during the outage is
indistinguishable from the noise — and the local gates cannot cover the gap, because the failures
that only appear in CI are precisely the ones a local run cannot produce. So **treat the first green
run after billing clears as the first real measurement of everything committed since
`2026-08-10T21:58:20Z`**, not as confirmation of what was already validated here. When a status is
untrustworthy, the reliable move is to find the last *successful* run and diff what it actually
contained, rather than reading the aggregate.

libro is not exposed to the external-host variant of this — GitHub Packages authenticates every
read and an off-GitHub builder sends anonymously, so an external host would fail install with
`ERR_PNPM_FETCH_401` while Actions stayed green on the same commit and lockfile. Verified here
rather than assumed, because a host can be connected with nothing committed to the repo: `0`
deployments, `0` environments, no host configuration in the tree, and the preview job is the
`artifact` provider rather than a hosting one. If an external host is ever connected, it needs its
own `.npmrc` and a **classic** PAT with `read:packages` — fine-grained tokens are not accepted.

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

**That capital-letter rule is a real hazard, not a stylistic note — reproduced against the upstream
checker.** The heuristic is `\(([A-Z][^)/#\n]{2,59})\)`, so *any* parenthetical starting with a
capital is treated as a claimed name, including glosses whose first word is a proper noun or an
acronym. Three sentence-shaped glosses libro would plausibly write all fail with a `claimed:`/
`actual:` diff on citations that are entirely correct:

```
ENG-INT-001   →  (Svelte components are the framework edge)
ENG-WEB-003   →  (LCP is judged against the SPA shell)
ENG-LOCAL-001 →  (IndexedDB is the system of record)
```

The arrows are load-bearing: the pattern requires the parenthetical to follow the ID directly, so
writing these examples in their natural adjacent form makes **this file** fail its own citation
gate. Documenting the hazard in the obvious way reproduces it.

libro passes today — 56 citations, 19 stated names matched, exit 0 — but only because its glosses
happen to start lowercase, which is luck rather than design. The failure presents as *"the checker
is broken"* on a correct citation, so it costs trust in the gate rather than catching anything.

The discriminator that separates the two populations is **membership in the title set**, not
capitalisation: a genuine miscitation names some *other* principle's real title, while a
descriptive gloss matches no title at all. Verified against `principles/index.json` — `Local
durable ownership` and `Minimal directed boundaries` are titles; all three glosses above are not.
Keep glosses lowercase here until the heuristic changes.

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
