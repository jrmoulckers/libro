/**
 * Pending manual conflicts and their resolution.
 *
 * In `manual` policy, {@link import('./sync').syncProgress} diverts a genuine,
 * unorderable conflict to a {@link ProgressConflict} instead of writing the store.
 * The {@link ConflictStore} holds the pending set (rebuilt each pass, keyed by
 * book id) for the UI; {@link resolveConflictProgress} is the pure choice→Progress
 * function, and {@link resolveProgressConflict} applies a chosen winner to the
 * correct lane store (and seeds anti-oscillation state so the next pass doesn't
 * ping-pong).
 */

import type { Progress } from '../models';
import type { SyncLane } from './lanes';
import type { ProgressStore } from './source';
import type { SyncMemory } from './sync';

/** The user's choice when resolving a {@link ProgressConflict}. */
export type ConflictChoice = 'keep_local' | 'use_remote' | 'keep_furthest';

/**
 * A conflict awaiting manual resolution: enough to render both options and, once
 * chosen, write the correct {@link Progress} into the correct lane. The two
 * concrete payloads are the values each choice would write (the remote side is
 * already merged to preserve local-only fields like an EPUB locator).
 */
export interface ProgressConflict {
  bookId: string;
  title: string;
  lane: SyncLane;
  /** Human label for the remote side, e.g. the ABS server's display name. */
  remoteSource: string;
  /** The concrete value "keep this device" would write. */
  local: Progress;
  /** The concrete value "use remote" would write. */
  remote: Progress;
}

/**
 * The concrete {@link Progress} a given {@link ConflictChoice} resolves to. Pure.
 * `keep_furthest` picks the larger fraction (ties keep local).
 */
export function resolveConflictProgress(
  conflict: ProgressConflict,
  choice: ConflictChoice,
): Progress {
  switch (choice) {
    case 'keep_local':
      return conflict.local;
    case 'use_remote':
      return conflict.remote;
    case 'keep_furthest':
      return conflict.remote.fraction > conflict.local.fraction ? conflict.remote : conflict.local;
  }
}

/**
 * The pending-conflict set for the current session, keyed by book id. Rebuilt on
 * each sync pass; the UI reads {@link list} and calls {@link resolveProgressConflict}
 * per row, which removes the entry via {@link clear}.
 */
export class ConflictStore {
  #map = new Map<string, ProgressConflict>();

  /** Replace the whole set (called at the end of a sync pass). */
  replaceAll(conflicts: readonly ProgressConflict[]): void {
    this.#map = new Map(conflicts.map((c) => [c.bookId, c]));
  }

  /** All pending conflicts, in insertion order. */
  list(): ProgressConflict[] {
    return [...this.#map.values()];
  }

  get(bookId: string): ProgressConflict | undefined {
    return this.#map.get(bookId);
  }

  size(): number {
    return this.#map.size;
  }

  clear(bookId: string): void {
    this.#map.delete(bookId);
  }

  clearAll(): void {
    this.#map.clear();
  }
}

/** Deps needed to apply a resolved conflict to the right lane. */
export interface ResolveDeps {
  /** Lane stores by lane, so the winner lands in reading vs. listening. */
  stores: Record<SyncLane, ProgressStore>;
  /** Anti-oscillation memory to seed so the chosen value isn't re-pushed. */
  memory?: SyncMemory;
}

/**
 * Apply the user's choice for one conflict: resolve it to a concrete
 * {@link Progress}, write it into the conflict's lane store, seed the
 * anti-oscillation memory, and return the value written. Failure-isolated at the
 * call site — the caller decides how to surface a store error.
 */
export async function resolveProgressConflict(
  conflict: ProgressConflict,
  choice: ConflictChoice,
  deps: ResolveDeps,
): Promise<Progress> {
  const winner = resolveConflictProgress(conflict, choice);
  await deps.stores[conflict.lane].put(conflict.bookId, winner);
  deps.memory?.noteSynced(conflict.bookId, winner);
  return winner;
}
