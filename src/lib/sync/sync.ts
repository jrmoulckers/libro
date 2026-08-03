/**
 * The two-way progress-sync sweep: pull each syncable book's remote progress,
 * reconcile it against the device-local lane store, and apply the winner —
 * writing pulled-down values locally and pushing local wins back up.
 *
 * A faithful browser port of the blueprint's `reconcile_catalog_with_policy`
 * (`core/src/progress_sync.rs`) plus the anti-oscillation seeding from
 * `core/src/listening_sync.rs`. Best-effort throughout: bounded-concurrency pulls,
 * `Promise.allSettled`-style isolation so one book's failure never aborts the
 * sweep, and it never throws. Everything but the injected source/store I/O is pure.
 *
 * ## Anti-oscillation
 *
 * After a pull-down (remote wins) or a manual resolution, {@link SyncMemory} is
 * seeded with the value just synced. On a later pass a `keep-local` outcome only
 * pushes when the local position has actually *moved* past the last synced mark
 * ({@link shouldPush}) — so a just-pulled value is never immediately echoed back
 * up, and the two directions can't ping-pong.
 */

import type { Book, Progress } from '../models';
import {
  reconcileProgress,
  mergeRemoteWin,
  PROGRESS_TIE_EPSILON,
  type ConflictResolution,
} from './reconcile';
import { laneForBook, type SyncLane } from './lanes';
import type { ProgressSource, ProgressStore } from './source';
import type { ProgressConflict } from './conflict';

/** A snapshot of the last value synced for a book, for echo suppression. */
export interface SyncedMark {
  fraction: number;
  finished: boolean;
}

/**
 * Decide whether a local position is worth pushing up, given the last value we
 * synced for it. Pure. Returns `true` when nothing was synced yet, when the
 * finished flag flipped, or when the fraction moved beyond
 * {@link PROGRESS_TIE_EPSILON}; `false` when the local value still matches the last
 * synced mark (suppress a redundant / echoed push).
 */
export function shouldPush(current: Progress, last: SyncedMark | undefined): boolean {
  if (!last) return true;
  if (current.finished !== last.finished) return true;
  return Math.abs(current.fraction - last.fraction) > PROGRESS_TIE_EPSILON;
}

/**
 * In-memory anti-oscillation state shared across sync passes. Not persisted: a
 * fresh reload starts empty, which is safe (the tie window still prevents thrash).
 */
export class SyncMemory {
  #map = new Map<string, SyncedMark>();

  /** Record the value just synced for a book (pull-down, push, or resolution). */
  noteSynced(bookId: string, progress: Progress): void {
    this.#map.set(bookId, { fraction: progress.fraction, finished: progress.finished });
  }

  get(bookId: string): SyncedMark | undefined {
    return this.#map.get(bookId);
  }
}

/** A tally of one sync sweep, surfaced to the caller/UI alongside the conflicts. */
export interface ReconcileReport {
  /** Remote won; a merged progress was written locally. */
  pulledDown: number;
  /** Local won and was pushed up to the remote. */
  pushed: number;
  /** Local won; no push needed (already in sync remotely). */
  keptLocal: number;
  /** The two agreed within thresholds; nothing written. */
  inSync: number;
  /** The remote had no record for the book. */
  noRemote: number;
  /** Genuine conflicts recorded as pending (manual mode); store untouched. */
  conflicts: ProgressConflict[];
  /** Per-book failures, swallowed so the sweep continues. */
  errors: { bookId: string; reason: string }[];
}

/** Everything {@link syncProgress} needs, all injectable so tests use fakes. */
export interface SyncDeps {
  /** The remote tracker to reconcile against (e.g. an ABS source). */
  source: ProgressSource;
  /** Local lane stores; a book routes to one via {@link laneForBook}. */
  stores: Record<SyncLane, ProgressStore>;
  /** `auto` (default) auto-resolves every case; `manual` surfaces genuine conflicts. */
  policy?: ConflictResolution;
  /** Anti-oscillation state; a fresh {@link SyncMemory} is used when omitted. */
  memory?: SyncMemory;
  /** Max concurrent remote pulls (default 4). */
  concurrency?: number;
  /**
   * Extra gate on which books to sync (default: all with a lane). The composition
   * root passes a predicate requiring a remote id the source recognizes.
   */
  isSyncable?: (book: Book) => boolean;
}

/** Run `fn` over `items` with at most `limit` in flight. Order-independent. */
async function mapPool<T, R>(
  items: readonly T[],
  limit: number,
  fn: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  const results = new Array<R>(items.length);
  let next = 0;
  const workers = Array.from({ length: Math.max(1, Math.min(limit, items.length)) }, async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await fn(items[index], index);
    }
  });
  await Promise.all(workers);
  return results;
}

interface Pulled {
  book: Book;
  lane: SyncLane;
  remote: Awaited<ReturnType<ProgressSource['pullProgress']>>;
  error?: string;
}

/**
 * Reconcile a catalog against a remote source and apply the winners. Never throws;
 * every fetch/store error is captured in {@link ReconcileReport.errors}. Books with
 * no lane, or excluded by `isSyncable`, are skipped silently.
 */
export async function syncProgress(
  books: readonly Book[],
  deps: SyncDeps,
): Promise<ReconcileReport> {
  const policy = deps.policy ?? 'auto';
  const memory = deps.memory ?? new SyncMemory();
  const gate = deps.isSyncable ?? (() => true);

  const report: ReconcileReport = {
    pulledDown: 0,
    pushed: 0,
    keptLocal: 0,
    inSync: 0,
    noRemote: 0,
    conflicts: [],
    errors: [],
  };

  // Build the work list: laned + gated books only.
  const work: { book: Book; lane: SyncLane }[] = [];
  for (const book of books) {
    const lane = laneForBook(book);
    if (lane && gate(book)) work.push({ book, lane });
  }

  // Phase 1 — bounded-concurrency remote pulls (read-only, network-bound).
  const fetched: Pulled[] = await mapPool(work, deps.concurrency ?? 4, async ({ book, lane }) => {
    try {
      return { book, lane, remote: await deps.source.pullProgress(book) };
    } catch (error) {
      return { book, lane, remote: null, error: reason(error) };
    }
  });

  // Phase 2 — sequential reconcile + apply (serialized store writes).
  for (const { book, lane, remote, error } of fetched) {
    if (error) {
      report.errors.push({ bookId: book.id, reason: error });
      continue;
    }
    if (!remote) {
      report.noRemote++;
      continue;
    }

    const store = deps.stores[lane];
    try {
      const local = await store.get(book.id);
      const decision = reconcileProgress(
        local?.progress ?? null,
        local?.updatedAt ?? null,
        remote,
        remote.updatedAt,
        policy,
      );

      switch (decision.kind) {
        case 'in-sync':
          report.inSync++;
          break;

        case 'conflict':
          // Manual mode only. `local` is guaranteed present when reconcile
          // returns a conflict (both sides diverged).
          report.conflicts.push({
            bookId: book.id,
            title: book.title,
            lane,
            remoteSource: deps.source.displayName,
            local: (local as NonNullable<typeof local>).progress,
            remote: mergeRemoteWin(remote, local?.progress ?? null),
          });
          break;

        case 'use-remote':
          await store.put(book.id, decision.progress);
          memory.noteSynced(book.id, decision.progress);
          report.pulledDown++;
          break;

        case 'keep-local': {
          const localProgress = local?.progress;
          if (localProgress && shouldPush(localProgress, memory.get(book.id))) {
            await deps.source.pushProgress(book, localProgress);
            memory.noteSynced(book.id, localProgress);
            report.pushed++;
          } else {
            report.keptLocal++;
          }
          break;
        }
      }
    } catch (err) {
      report.errors.push({ bookId: book.id, reason: reason(err) });
    }
  }

  return report;
}

function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
