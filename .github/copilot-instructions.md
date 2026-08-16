<!-- studio:base:start -->
<!-- synced from jrmoulckers/.github — canonical source; do not edit here -->

# GitHub Copilot — JRM Studio orientation

This file orients GitHub Copilot on the **Copilot surfaces**: chat and completions in VS Code,
`copilot.com`, code review, and the coding agent.

**`AGENTS.md` in the repository root is the authoritative operating guide.** Read it before acting.
It owns the golden rules, the Definition of Done, the issue-first workflow, and the mandatory
human-gated operations. This file never restates those rules — where the two could appear to
conflict, `AGENTS.md` wins.

> Distributed from `jrmoulckers/.github`. Everything outside the `studio:base` markers is
> repository-local and is never overwritten by the studio sync tool; add repository-specific
> Copilot orientation there rather than editing inside the managed region.

## Read order

1. **`AGENTS.md`** (root) — the operating guide. Product repositories extend it below the managed
   region; those local rules layer on top of the shared floor but never relax a human gate.
2. **The nearest scoped instructions** — `.github/instructions/*.instructions.md` apply by glob to
   the paths you are editing. A more specific file wins over a more general one.
3. **Product context** — whatever the repository's own `AGENTS.md` section points you to
   (architecture notes, ADRs, design or domain docs).

## The installed AI layer

Much of `.github/` is generated and distributed from the `jrmoulckers/.github` backbone:

| Path | What it is | How to use it |
| --- | --- | --- |
| `.github/agents/*.agent.md` | Role definitions with explicit boundaries | Delegate specialist work to the matching role instead of improvising one |
| `.github/skills/<name>/SKILL.md` | Reusable methodology and checklists | Consult the skill whose description matches the task before inventing an approach |
| `.github/prompts/*.prompt.md` | Repeatable multi-step workflows | Prefer the existing prompt over an ad-hoc plan for the work it covers |
| `.github/instructions/*.instructions.md` | Path-scoped rules | Applied automatically by glob; obey the most specific match |
| `agency.toml` | Reviewed MCP servers and tool allowlists | Do not add servers or widen tool grants locally |

**Provenance is per file, not per directory — and in some files, per region.** Three shapes:

- **Whole-file canon.** A `synced from jrmoulckers/.github` marker at the top and no region markers.
  The entire file is generated. The comment syntax varies with the file type — HTML in Markdown, `#`
  in `.toml`, `.yml`, `.gitattributes` and `.gitignore`, `/* */` in `.js`, `.ts`, `.css`, `.kt` and
  `.swift`, and none at all in `.json`, which has no comment syntax.
- **Managed-region files.** Root `AGENTS.md`, this file, and `.gitattributes` carry canon *between*
  the `studio:base:start` and `studio:base:end` markers and are member-owned everywhere else. The
  block is generated; the surrounding content is yours to write, trim, and maintain. Editing inside
  the markers is drift; editing outside them is expected. `sync/lib/copier.mjs` is authoritative for
  which files these are — the list above is illustrative and grows when a managed-merge kind is
  added.
- **Unmarked files.** Repository-owned and yours to edit normally. `.github/agents/` in particular
  routinely holds both tiers side by side: canonical studio roles alongside locally authored agents
  carrying authority specific to that repository.

Check the marker — and, in a managed-region file, which side of it you are on — before assuming
anything is off-limits.

**Never edit generated content to change shared behaviour.** A local edit is detected as drift, is
skipped on the next sync, and silently strands the repository on a stale copy. Change the canonical
source in `jrmoulckers/.github` and let it sync. Genuinely repository-specific behaviour belongs in
the local `AGENTS.md`, a locally authored agent, or a scoped instructions file.

## Checking a managed region

Generated files carry `synced from jrmoulckers/.github — canonical source; do not edit here`
between markers. Everything **outside** those markers is yours: the engine splices its region in
and preserves the rest, so local content there is expected, not drift.

Ask the engine rather than comparing by hand — it is the same code that performs the write:

```bash
node sync/index.mjs --dry-run --members <member-name> --work-dir /path/to/your/checkout
```

Run it from a backbone checkout with full history: it refuses on a shallow clone rather than
comparing against truncated canon. Fetch that checkout first. The engine reads canon off your disk
and never compares it to its own origin, so **depth is guarded and currency is not**: a complete but
stale backbone reports every target unchanged while canon has moved on.

**Read the answer as repo-wide, because that is what it is.** The report is a count across every
target in your repo — `unchanged: 58`, `updated: 1`, `→ changes pending` — and there is no per-file
or verbose flag. All-unchanged means every one of your files already matches canon *as of the
backbone checkout you ran it from*, and is a complete answer for that revision. A non-zero `updated`
means a re-sync brings them into line with no hand editing,
but it does **not** tell you which target moved, so it cannot confirm or clear the specific region
you came to check. Treat it as a repo-level verdict and re-run it after the sync lands.

**Do not diff the whole file.** `AGENTS.md`, `.github/copilot-instructions.md` and `.gitattributes`
are spliced rather than copied, so a whole-file comparison against canon reports drift on every
*correctly* synced repo — it is measuring your own local content. Compare the region between the
markers, or use the command above. If the region really is stale or damaged, the fix is a backbone
change or a re-sync, never a hand edit: the next run overwrites it.

## Working conventions

- **Issue first, PR always.** Read-only research needs no issue; the requirement starts before your
  first change. A change that ends at a local commit is not done.
- **Reference, never restate.** Each rule has exactly one canonical owner. Link to it rather than
  copying its text — duplicated policy drifts and hides which copy is authoritative.
- **Stay in scope.** Surgical edits only. Do not reformat or refactor code the task did not require.
- **Verify before claiming done.** Run the repository's own lint, type-check, test, and build
  commands. Do not report success on unverified work.
- **Say when you are unsure.** A short clarifying question beats a confident guess on anything
  touching security, privacy, data, or infrastructure.

## Talking to other sessions

Several sessions work this fleet at once, they authenticate as the same account, and their messages
arrive interleaved in each other's inboxes. Undifferentiated, that reads as a single voice
contradicting itself. The usual symptom is a correspondent repeating an answered request, because
the acknowledgement they got was overwritten by someone else's continued asking.

- **Identify yourself.** Open every cross-session message with `**[from: owner/repo]**`. One line,
  and it is what makes "whose question is this" answerable at the receiver.
- **Attribute from the receipt, not from memory.** Before crediting, disputing, or disowning a
  claim, find it in the transcript. Recalled attribution has been wrong in both directions here,
  including a session confidently disowning a message that was verbatim its own.
- **A null about your own past words is weak evidence.** The search terms come from the memory
  already under suspicion, and a query returning nothing is indistinguishable from one that was
  never capable of returning anything. Both failed here inside a day: one session missed its own
  message by searching vocabulary it only acquired afterwards, and another read a truncated
  fleet-wide result as absence in its own session. A recipient's verbatim quote outranks your null.
- **A control must share the probe's defect surface.** A control run in a different shape from the
  probe it validates cannot detect what the probe got wrong — the one above returned a healthy
  count while querying with a filter the probe lacked, so it could only confirm the store was
  reachable, which was never in question. A control that fires correctly and is blind to the defect
  is worse than one that cannot fire, because firing reads as validation.
- **A repeated request means a broken channel, not an idle correspondent.** When something you
  already answered is asked again, resend the answer with its issue or commit identifier rather
  than restating the reasoning; the identifier is checkable and the reasoning is not.
- **Re-run measurements rather than re-quoting them.** A sync verdict describes one commit of a
  repository that moves hourly. Name the SHA you measured at, and re-measure before acting on any
  verdict older than a few hours — including your own. This cures staleness only: repeated readings
  that all sit downstream of the change you are crediting agree with each other and establish
  nothing. See *Repeating a measurement is not a control* in `workflow.instructions.md`.

## Stop and ask

`AGENTS.md` §"Human-Gated Operations" is the complete, canonical list. Pushing your own feature
branch and opening a PR are *required* steps — never pause for permission on those. Do stop for
anything that writes to a shared branch, acts on another author's PR, changes repository settings,
touches real secrets or credentials, reaches outside the repository root, deletes files in bulk, or
publishes or deploys.

When a task needs a gated operation and no human is available, finish everything that is
auto-approved and leave a `## Needs Human Action` note describing exactly what remains and why.
<!-- studio:base:end -->
