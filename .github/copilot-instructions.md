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

Most of `.github/` is generated and distributed from the `jrmoulckers/.github` backbone. Treat it
as read-only here:

| Path | What it is | How to use it |
| --- | --- | --- |
| `.github/agents/*.agent.md` | Role definitions with explicit boundaries | Delegate specialist work to the matching role instead of improvising one |
| `.github/skills/<name>/SKILL.md` | Reusable methodology and checklists | Consult the skill whose description matches the task before inventing an approach |
| `.github/prompts/*.prompt.md` | Repeatable multi-step workflows | Prefer the existing prompt over an ad-hoc plan for the work it covers |
| `.github/instructions/*.instructions.md` | Path-scoped rules | Applied automatically by glob; obey the most specific match |
| `agency.toml` | Reviewed MCP servers and tool allowlists | Do not add servers or widen tool grants locally |

**Never edit a generated file to change shared behaviour.** A local edit is detected as drift, is
skipped on the next sync, and silently strands the repository on a stale copy. Change the canonical
source in `jrmoulckers/.github` and let it sync. Genuinely repository-specific behaviour belongs in
the local `AGENTS.md`, a locally authored agent, or a scoped instructions file.

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

## Stop and ask

`AGENTS.md` §"Human-Gated Operations" is the complete, canonical list. Pushing your own feature
branch and opening a PR are *required* steps — never pause for permission on those. Do stop for
anything that writes to a shared branch, acts on another author's PR, changes repository settings,
touches real secrets or credentials, reaches outside the repository root, deletes files in bulk, or
publishes or deploys.

When a task needs a gated operation and no human is available, finish everything that is
auto-approved and leave a `## Needs Human Action` note describing exactly what remains and why.
<!-- studio:base:end -->
