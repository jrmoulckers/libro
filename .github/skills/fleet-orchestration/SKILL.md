---
name: fleet-orchestration
description: >
  Parallel multi-agent execution across isolated worktrees. Use for topics
  related to dispatching several agents at once, wave and sprint dispatch,
  worktree coordination, file-ownership conflicts, parallel PR workflows,
  CI self-healing loops, rebase-all maintenance, or merge ordering.
---
<!-- synced from jrmoulckers/.github — canonical source; do not edit here -->

# Fleet Orchestration Skill

## Purpose

This skill covers **executing already-planned work in parallel**: dispatching multiple agents,
isolating them in worktrees, keeping them from colliding, healing their CI failures, and landing
their pull requests in a safe order.

It assumes issues are already shaped and scope is already selected. It is about *running* a wave,
not deciding what the wave should contain.

## Out of Scope

- Selecting and sequencing the work before dispatch → use `sprint-planning`.
- Roadmap, milestones, backlog grooming, and release lifecycle → use `project-management`.
- Issue quality, labels, and duplicate detection before filing → use `issue-management`.
- The push/PR/merge command sequence itself → owned by `AGENTS.md` and the repository's own
  workflow documentation. Never restate it here; a stale copy causes avoidable CI failures.
- Domain implementation inside any app, package, or service → use the relevant engineering skill.

## Related Skills

| Skill | Use for |
| --- | --- |
| `sprint-planning` | Selecting and sequencing work before dispatch |
| `project-management` | Lifecycle and release tracking across a fleet |
| `issue-management` | Issue quality, scoping, labels, and duplicates |
| `dev-onboarding` | Environment setup and local tooling orientation |

## When a fleet is the wrong tool

Parallelism has real overhead: worktree setup, CI contention, merge-order reasoning, and rebase
churn. Dispatch a fleet only when **all** of these hold.

| Condition | Why it matters |
| --- | --- |
| Three or more genuinely independent tasks | Below that, sequential work is faster than coordination |
| Disjoint file ownership | Overlapping edits serialize anyway, with conflicts on top |
| Each task is individually shippable | A fleet cannot land a change that only makes sense as one PR |
| Scope is already decided | Planning mid-flight desynchronizes every agent at once |

A single background agent is not a fleet. For solo work, run it synchronously.

## Dispatch algorithm

### 1. Enumerate the candidate work

```bash
gh issue list --state open --json number,title,labels,milestone --limit 100
```

### 2. Route each item to an owner

Map by label where a taxonomy exists, otherwise infer from the issue body. Route to the role whose
`.github/agents/*.agent.md` boundary actually covers the files that must change — ownership is
defined by the agent definitions, not by this skill.

### 3. Serialize the dependencies

Chains that must never run in parallel:

| Rule | Reason |
| --- | --- |
| Schema before its consumers | Clients cannot compile against a shape that does not exist |
| Shared models before leaf platforms | Every platform consumer reads the shared definition |
| Design tokens before UI that uses them | Generated token output is an input to the UI build |
| Architecture decision before implementation | An ADR reversal invalidates finished work |

Cross-layer schema changes are **one serialized task**, not two independent ones.

### 4. Balance into waves

- Prefer a wave you can hold in your head over a maximal one.
- Order by priority: bugs → security → features → docs → chores.
- Budget roughly a fifth of wave time for CI failures and rebases; a wave with zero slack stalls.
- Reserve capacity for non-engineering work so triage and docs do not starve.

### 5. Track dependencies explicitly

```sql
INSERT INTO todos (id, title, description, status) VALUES
  ('w1-shared-88', 'Updating shared models (#88)', 'Shared model change other tasks depend on', 'pending'),
  ('w1-web-443',   'Building the dashboard (#443)', 'Consumes the shared models from #88', 'pending');

INSERT INTO todo_deps (todo_id, depends_on) VALUES ('w1-web-443', 'w1-shared-88');
```

Query for dispatchable work — pending with no unfinished dependency:

```sql
SELECT t.id, t.title FROM todos t
WHERE t.status = 'pending'
  AND NOT EXISTS (
    SELECT 1 FROM todo_deps td
    JOIN todos dep ON td.depends_on = dep.id
    WHERE td.todo_id = t.id AND dep.status != 'done'
  );
```

## Dispatch pattern

Launch every independent agent in the same turn, then wait. Dispatching serially forfeits the only
benefit of a fleet.

```
# One turn, all independents:
task(name: "w1-shared-88", agent_type: "…", mode: "background", prompt: …)
task(name: "w1-web-443",   agent_type: "…", mode: "background", prompt: …)
task(name: "w1-docs-446",  agent_type: "…", mode: "background", prompt: …)
```

For a dependency chain: dispatch the independents, collect results, then dispatch the dependents.

Each agent prompt must be self-contained, because agents share no context:

- The issue number, title, and full body — not a reference to it.
- The worktree it owns and the branch to create.
- The files it owns, and an explicit instruction not to touch others.
- A pointer to the repository's canonical pre-push and PR workflow — never an inline copy.
- The completion bar: PR open, CI green, `MERGEABLE`, then self-merge.

## Worktree protocol

One agent, one worktree, one branch. Sharing a worktree is the single largest source of fleet
failure: agents overwrite each other's checkouts and interleave commits onto the wrong branch.

```bash
git worktree add <worktrees-dir>/<agent>-<issue> -b <type>/<description>-<issue>
# … agent works, pushes, opens a PR, merges …
git worktree remove <worktrees-dir>/<agent>-<issue>
```

Clean up merged worktrees at the end of each wave. Abandoned worktrees accumulate stale branches
that later rebases must repeatedly reconcile.

## Parallel coordination rules

**File ownership is exclusive.** No two agents in a wave may edit the same file. If a file genuinely
needs input from two roles, one owns the edit and the other reviews.

**Shared configuration gets a single owner per wave.** Dependency manifests, lockfiles, lint
configuration, and build configuration are edited by exactly one agent per wave, whichever role owns
build and delivery. Two agents editing a lockfile in parallel conflict on essentially every run.

**Ordered layers stay serialized.** Where a change crosses a schema or contract boundary, plan it as
one task with ordered steps rather than parallel tasks with a hopeful merge order.

## CI self-healing loop

```
push → check CI status and mergeability
     → green and MERGEABLE?  → self-merge
     → otherwise             → read the failing logs
                             → fix locally
                             → re-run the canonical pre-push sequence
                             → push, repeat
```

| Failure | Response |
| --- | --- |
| Format or lint | Run the repository's format/lint fix command, commit, push |
| Type error | Fix the reported error, re-run the type-check, push |
| Test failure | Fix the test or the code, re-run the affected tests, push |
| Merge conflict | Fetch and rebase onto the default branch, resolve, push |

A conflicted PR is as blocking as a red one. Both must be cleared before merge.

## Rebase-all maintenance

When the default branch advances mid-wave, reconcile **one worktree at a time**. Batch-rebasing in
parallel produces interleaved conflict resolutions that are hard to attribute and easy to get wrong.

For each agent-owned branch: fetch, rebase onto the updated default branch, resolve conflicts, run
the canonical pre-push sequence, then push. `--force-with-lease` is appropriate here — but only on
the agent's own branch, and only after a successful rebase.

## Execution phases

| Phase | Activity | Exit condition |
| --- | --- | --- |
| **Plan** | Enumerate, route, serialize dependencies, record todos | Every task has an owner and known dependencies |
| **Dispatch** | Launch all independents in one turn | Every independent agent is running |
| **Monitor** | Collect results, drive CI to green, update todo status | No agent is silently stuck |
| **Validate** | Confirm every PR is open, green, and `MERGEABLE` | The full check suite passes from a clean checkout |
| **Handoff** | Merge in dependency order, clean up worktrees | Nothing remains only on a side branch |

Merge in dependency order, never in completion order: a dependent PR merged before its prerequisite
either breaks the default branch or silently reintroduces the old shape.

## Hard-won lessons

| Lesson | Detail |
| --- | --- |
| Never share a worktree | Branch interference is the top cause of fleet failure |
| Rebase immediately before pushing | Stale branches compound conflicts across the whole wave |
| Follow the canonical pre-push sequence | Copied or remembered command sequences go stale and fail CI |
| Treat warnings as errors | Warnings accumulate silently across parallel PRs until CI rejects them |
| Every PR meets the same bar | Docs and chore PRs follow the same gate as feature PRs |
| A wave is done when it is merged | Green PRs left unmerged are unfinished work, not finished work |

## Acceptance checklist

- [ ] Every dispatched task had an owner, a worktree, and disjoint file ownership.
- [ ] Dependencies were serialized, not merged optimistically.
- [ ] Every PR references its issue and satisfies the repository's Definition of Done.
- [ ] Every PR is green **and** `MERGEABLE` before any merge.
- [ ] PRs merged in dependency order.
- [ ] Worktrees and merged branches cleaned up.
- [ ] Anything blocked by a human gate left exactly one `## Needs Human Action` note.
