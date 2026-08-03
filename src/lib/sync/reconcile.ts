/**
 * Pure two-way progress reconciliation — the deterministic decision engine that
 * merges a device-local reading/listening position with a remote tracker's
 * (e.g. Audiobookshelf) current progress.
 *
 * This is a faithful port of the blueprint's `core/src/progress_sync.rs`
 * `reconcile` / `is_genuine_conflict` policy, re-expressed as pure, DOM-free,
 * network-free TypeScript so the entire truth table is unit-testable without a
 * browser, a store, or a fetch.
 *
 * ## Conflict policy (precise, mirrors the blueprint)
 *
 * {@link reconcileProgress} decides a winner as follows:
 *  1. **`finished` is sticky.** Once an item is finished on *either* side it stays
 *     finished — a stale in-progress record never un-finishes it. A finished side
 *     is therefore always a deterministic winner, never a conflict.
 *  2. **Tie window — no thrash.** Fractions within {@link PROGRESS_TIE_EPSILON}
 *     are treated as already in sync, so a negligible delta never writes.
 *  3. **Newest-wins by timestamp.** When *both* sides carry an `updatedAt` and they
 *     differ by more than {@link RECENCY_TIE_SECONDS}, the newer side wins
 *     (last-write-wins — it may even rewind, a deliberate re-read on another device).
 *  4. **Fallback.** Otherwise (divergent positions, neither finished, timestamps
 *     missing or within the tie window) the two cannot be confidently ordered:
 *     under the `manual` policy this surfaces as a {@link isGenuineConflict genuine
 *     conflict}; under `auto` it resolves deterministically by furthest-position.
 */

import type { Progress } from '../models';

/**
 * Fractions within this absolute delta are considered "the same position", so a
 * negligible difference never causes a store write (no-thrash). Mirrors the
 * blueprint's `PROGRESS_TIE_EPSILON`.
 */
export const PROGRESS_TIE_EPSILON = 0.01;

/**
 * When both sides carry an `updatedAt`, timestamps closer than this (seconds) are
 * treated as a tie and fall back to furthest-position. Mirrors the blueprint's
 * `RECENCY_TIE_SECONDS`.
 */
export const RECENCY_TIE_SECONDS = 2;

/** A book counts as finished a hair before 1.0 to tolerate rounding. */
const FINISHED_FRACTION = 0.99;

/**
 * A normalized snapshot of a book's progress on a *remote* service, produced by a
 * {@link import('./source').ProgressSource} from its own API shape.
 *
 * It is structurally a {@link Progress} (so it flows through the same model) plus
 * an `updatedAt` server timestamp that drives the newest-wins branch. The local
 * {@link Progress} model carries no per-write timestamp yet, so the pass passes
 * `null` for the local time and unfinished ties fall back to furthest-position —
 * exactly as the blueprint documents, with the newest-wins branch ready the moment
 * a timestamped local store lands.
 */
export interface RemoteProgress extends Progress {
  /** Server last-update time (epoch **seconds**), or `null` when the remote omits it. */
  updatedAt: number | null;
}

/** How ambiguous conflicts are handled. `auto` never produces pending conflicts. */
export type ConflictResolution = 'auto' | 'manual';

/**
 * The outcome of reconciling one book's local vs. remote progress.
 *
 *  - `in-sync` — the two agree (within thresholds), or nothing to do; no write.
 *  - `keep-local` — the local value is authoritative; leave the store untouched.
 *  - `use-remote` — the remote is authoritative; write `progress` (already merged
 *    to preserve local fields the remote lacks, e.g. an EPUB locator).
 *  - `conflict` — (manual policy only) divergent and unorderable; surface to the
 *    user, do not write.
 */
export type ReconcileDecision =
  | { kind: 'in-sync' }
  | { kind: 'keep-local' }
  | { kind: 'use-remote'; progress: Progress }
  | { kind: 'conflict' };

/** Whether a progress record counts as finished (explicit flag or ~complete). */
export function isProgressFinished(p: Progress): boolean {
  return p.finished || p.fraction >= FINISHED_FRACTION;
}

/**
 * Build the {@link Progress} to store when the remote wins, merging in local
 * fields the remote doesn't carry: remote trackers have no EPUB locator, and
 * status-only sources have no seconds — so keep the local `locator` /
 * `positionSeconds` as a fallback rather than dropping the reader's precise resume
 * point. Pure. Mirrors the blueprint's `remote_to_progress`.
 */
export function mergeRemoteWin(remote: Progress, local: Progress | null): Progress {
  const finished = isProgressFinished(remote);
  const merged: Progress = {
    fraction: finished ? 1 : remote.fraction,
    finished,
  };
  const positionSeconds = remote.positionSeconds ?? local?.positionSeconds;
  if (positionSeconds !== undefined) merged.positionSeconds = positionSeconds;
  const locator = local?.locator;
  if (locator !== undefined) merged.locator = locator;
  return merged;
}

/**
 * Reconcile a local and a remote progress under the module policy. Pure; never
 * throws. Both `updatedAt` args are epoch **seconds** (or `null` when unknown).
 *
 * Under `auto` (default) a divergent, unorderable pair resolves deterministically
 * by furthest-position — so switching policy never regresses the unambiguous
 * cases. Under `manual` that same pair returns `conflict` for the user to resolve;
 * every clear winner still auto-applies.
 */
export function reconcileProgress(
  local: Progress | null,
  localUpdatedAt: number | null,
  remote: Progress | null,
  remoteUpdatedAt: number | null,
  policy: ConflictResolution = 'auto',
): ReconcileDecision {
  if (!local && !remote) return { kind: 'in-sync' };
  if (local && !remote) return { kind: 'keep-local' };
  if (!local && remote) return { kind: 'use-remote', progress: mergeRemoteWin(remote, null) };

  // Both present — the interesting case.
  const l = local as Progress;
  const r = remote as Progress;
  const lFin = isProgressFinished(l);
  const rFin = isProgressFinished(r);

  // 1. finished is sticky.
  if (lFin && rFin) return { kind: 'in-sync' };
  if (lFin) return { kind: 'keep-local' }; // don't un-finish locally
  if (rFin) return { kind: 'use-remote', progress: mergeRemoteWin(r, l) };

  // 2. tie window — negligible delta is already in sync (no thrash).
  if (Math.abs(l.fraction - r.fraction) <= PROGRESS_TIE_EPSILON) return { kind: 'in-sync' };

  // 3. newest-wins when both timestamps are present and meaningfully apart.
  if (
    localUpdatedAt !== null &&
    remoteUpdatedAt !== null &&
    Math.abs(localUpdatedAt - remoteUpdatedAt) > RECENCY_TIE_SECONDS
  ) {
    return remoteUpdatedAt > localUpdatedAt
      ? { kind: 'use-remote', progress: mergeRemoteWin(r, l) }
      : { kind: 'keep-local' };
  }

  // 4. divergent, neither finished, no confident ordering ⇒ genuine conflict.
  if (policy === 'manual') return { kind: 'conflict' };

  // auto fallback: furthest-position-wins (with the no-thrash tie window).
  if (r.fraction - l.fraction > PROGRESS_TIE_EPSILON) {
    return { kind: 'use-remote', progress: mergeRemoteWin(r, l) };
  }
  if (l.fraction - r.fraction > PROGRESS_TIE_EPSILON) return { kind: 'keep-local' };
  return { kind: 'in-sync' };
}

/**
 * Detect a **genuine, unorderable** conflict — the only case a `manual` policy
 * surfaces. Returns `true` only when the fractions diverge beyond
 * {@link PROGRESS_TIE_EPSILON}, neither side is finished, and the timestamps
 * cannot confidently order the two (either missing, or within
 * {@link RECENCY_TIE_SECONDS}). A clear winner (one side finished, one clearly
 * newer, or within the tie window) returns `false` and still auto-resolves.
 *
 * Pure classifier, no side effects. `reconcileProgress(..., 'manual')` returns a
 * `conflict` decision exactly when this returns `true`.
 */
export function isGenuineConflict(
  local: Progress,
  localUpdatedAt: number | null,
  remote: Progress,
  remoteUpdatedAt: number | null,
): boolean {
  // Finished-sticky resolves deterministically — never a conflict.
  if (isProgressFinished(local) || isProgressFinished(remote)) return false;
  // Positions within the no-thrash window are "in sync" — never a conflict.
  if (Math.abs(local.fraction - remote.fraction) <= PROGRESS_TIE_EPSILON) return false;
  // Both timestamps present and meaningfully apart ⇒ newest-wins can decide.
  if (
    localUpdatedAt !== null &&
    remoteUpdatedAt !== null &&
    Math.abs(localUpdatedAt - remoteUpdatedAt) > RECENCY_TIE_SECONDS
  ) {
    return false;
  }
  // Divergent positions with no confident ordering ⇒ genuine conflict.
  return true;
}
