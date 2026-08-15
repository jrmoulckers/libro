---
name: edge-sync
description: >
  Offline-first client architecture and data synchronization. Use for topics
  related to local-first reads and writes, offline mutation queues, delta or
  incremental sync, conflict resolution strategies, tombstones and soft
  deletes, sync-state UI, replication, or edge resilience.
---
<!-- synced from jrmoulckers/.github — canonical source; do not edit here -->

# Edge Sync Skill

## Purpose

This skill covers **client-side offline-first behavior**: treating a local store as the primary
read/write surface, queueing mutations while disconnected, reconciling divergent copies, and
surfacing sync state honestly in the UI.

It is stack-agnostic. The patterns apply equally to a browser app over IndexedDB, a native app over
SQLite, or any client that must stay usable without a network.

## Out of Scope

- Server schema, migrations, row-level authorization, and replication rules → owned by the backend
  or database role for the repository.
- Platform build systems, source-set layout, and ORM syntax → use the relevant engineering skill.
- Threat modeling of the auth and encryption paths → use `security-review-methodology`.
- Privacy obligations and retention rules that sync must satisfy → use `privacy-compliance`.

## Related Skills

| Skill | Use for |
| --- | --- |
| `security-review-methodology` | Reviewing encrypted sync, token handling, and conflict paths |
| `privacy-compliance` | Erasure, retention, and data-residency obligations sync must honor |
| `performance-budgets` | Cost of sync work on the main thread and on startup |
| `ux-testing` | Validating that offline and conflict states are understandable |

## The core principle

**Every read and write goes to the local store. The network is only for synchronization.**

```
┌──────────────── Client (edge) ─────────────────┐
│  UI  ⇄  Repositories  ⇄  Local store           │
│                            ↕                    │
│                     Mutation queue              │
│                            ↕                    │
│                       Sync engine               │
└────────────────────────────┬───────────────────┘
                             │  (only crossing)
┌────────────────────────────┴───────────────────┐
│  Server: source of truth, authorization,        │
│  selective replication per authorized scope     │
└────────────────────────────────────────────────┘
```

If a UI code path can block on the network, the app is not offline-first. A write must be visible in
the local store — and therefore on screen — before any request is attempted.

## Sync engine states

Model the connection as an explicit state machine and expose it to the UI:

```
Disconnected → Connecting → Connected → Syncing → Connected
     ↑                                      │
     └──────────────── Error ←──────────────┘
```

| State | UI obligation |
| --- | --- |
| `Connected` | No indicator; this is the normal case |
| `Syncing` | Subtle, non-blocking progress; never a modal |
| `Disconnected` | Persistent but unobtrusive offline affordance; the app stays fully usable |
| `Error` | Explain what failed and offer retry; never fail silently |

Never block interaction on a sync state. Offline is a normal operating mode, not an error.

## Offline mutation queue

Local writes are recorded in a durable, ordered queue and replayed when connectivity returns.

**Requirements:**

- **Durable.** The queue survives a reload or process kill. An in-memory queue silently discards
  user work on a crash.
- **Ordered.** Replay preserves causal order; a delete replayed before its create corrupts state.
- **Deduplicated.** Repeated edits to one entity collapse by key rather than replaying every
  keystroke.
- **Bounded retry.** Exponential backoff with a ceiling, then a dead-letter state. Infinite retry of
  a permanently invalid mutation blocks every mutation behind it.
- **Observable.** A dead-lettered mutation is surfaced to the user, never dropped quietly.

**Backoff sketch:**

```
attempt 1 → immediate
attempt n → min(base × 2^(n-1), cap) + jitter
n > ceiling → dead-letter, surface to the user
```

Jitter matters: without it, every client that lost connectivity in the same outage retries in
lockstep and stampedes the server on recovery.

## Delta sync

Full re-downloads do not scale. Sync incrementally and verify integrity:

- Track a per-collection sequence or cursor.
- Detect gaps in the sequence rather than assuming contiguity.
- Validate a checksum before advancing the stored cursor. Advancing on unverified data makes
  corruption permanent, because the client will never request that range again.
- On mismatch, fall back to a scoped resync of the affected collection, escalating to a full resync
  only when scoped recovery fails.

## Soft deletes and tombstones

A hard delete cannot propagate: an absent row is indistinguishable from a row the client has not
seen yet.

- Delete by marking, not removing.
- Every replication query filters out deleted records — one unfiltered query resurrects deleted data
  on every client.
- Retain tombstones long enough for the slowest realistic client to reconnect; purge on a documented
  schedule, not never.

## Conflict resolution

Choose a strategy per collection, deliberately. There is no universally correct default.

| Strategy | Use for | Behavior | Main risk |
| --- | --- | --- | --- |
| **Last-write-wins** | Simple independent records | Newest timestamp wins | Silent loss of a concurrent edit |
| **Field-level merge** | Records edited by several people | Reconcile per field; flag genuine collisions | Complexity; needs per-field timestamps |
| **Client-wins** | Device-local preferences | Local copy always wins | Never converges across devices |
| **Server-wins** | Administrative or derived data | Remote copy always wins | Discards offline user work |

**Guidance:**

- Default to last-write-wins only for records a single user edits on one device at a time.
- Use field-level merge for anything collaborative; two people editing different fields of the same
  record is a merge, not a conflict.
- Last-write-wins on clock comparison is only as trustworthy as the clocks. Prefer a server-assigned
  version or a logical clock; device clocks skew, drift, and are user-settable.
- Never resolve a conflict by silently discarding a user's work when the loss is visible to them —
  surface it and let them choose.

## Encryption and erasure

When sync carries sensitive fields:

- Encrypt sensitive fields client-side before they enter the queue, so plaintext never crosses the
  network or lands in server storage.
- Use an envelope scheme: a per-scope data key wrapped by a rotatable master key. Rotating the
  wrapping key must not require re-encrypting every record.
- Support **crypto-shredding** — destroying a scope's data key renders its records unrecoverable
  everywhere, which is what makes erasure enforceable across replicas you cannot reach.
- Never log record contents, keys, or tokens from sync paths. Sync code runs constantly and is a
  prolific source of accidental data leaks.

## Common patterns

### Adding a new synchronized collection

1. Confirm the server includes the collection in the correct authorization scope.
2. Confirm every replication query filters deleted records.
3. Choose and register the conflict strategy explicitly — do not inherit a default by accident.
4. Add mutation-queue handling, including retry and dead-letter behavior for its writes.
5. Verify an offline write appears in the UI immediately and survives a reload.
6. Verify a two-device divergence converges to the intended result.

### Testing offline behavior

Sync bugs surface only under conditions that never occur on a fast desktop connection. Test:

| Scenario | What it catches |
| --- | --- |
| Write offline, reload, reconnect | Queue durability |
| Two devices edit the same record while offline | Conflict strategy correctness |
| Reconnect mid-sync | Partial-batch and cursor-advance handling |
| Corrupted or gapped sequence | Checksum validation and resync recovery |
| Permanently rejected mutation | Dead-lettering instead of head-of-line blocking |
| Delete on one device, edit on another | Tombstone propagation and resurrection bugs |

Multi-device convergence needs a harness that can run two simulated clients against one backend.
Manual testing reliably misses this class of bug.

## Acceptance checklist

- [ ] Every read and write path targets the local store; no UI path blocks on the network.
- [ ] The mutation queue is durable, ordered, deduplicated, and bounded by a retry ceiling.
- [ ] Dead-lettered mutations are surfaced rather than dropped.
- [ ] Sync cursors advance only after integrity validation.
- [ ] Deletes are soft, filtered from replication, and their tombstones have a documented lifetime.
- [ ] Each collection has a deliberately chosen conflict strategy.
- [ ] Conflict resolution does not rely on unverified device clocks.
- [ ] Sensitive fields are encrypted before entering the queue; erasure is enforceable.
- [ ] Sync paths log no record contents, keys, or tokens.
- [ ] Offline, reconnect, and two-device divergence are covered by automated tests.
