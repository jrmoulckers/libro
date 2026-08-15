/**
 * The remote/local surfaces the progress-sync pass reconciles between.
 *
 * A {@link ProgressSource} is the *remote* tracker (Audiobookshelf today, other
 * connectors later): it exposes the current progress for a book and accepts a
 * pushed-up position. A {@link ProgressStore} is the *local* device store for one
 * lane (reading or listening); the lane adapters in `./lanes` implement it over
 * the P5 {@link import('../reader/reading-store').ReadingStore} and P6
 * {@link import('../player/listening-store').ListeningStore}.
 *
 * Both are deps-injected into {@link import('./sync').syncProgress}, so the whole
 * orchestration is exercised in tests with in-memory fakes — no network, no DOM.
 */

import type { Book, Progress } from '../models';
import type { RemoteProgress } from './reconcile';

/** Typed error for remote progress-sync fetch/parse/push failures. */
export class SyncError extends Error {
  constructor(
    message: string,
    override readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'SyncError';
  }
}

/**
 * The minimal remote surface the reconciliation pass needs: pull a book's current
 * remote progress (or `null` when the remote has no record), and push a local
 * position up. Best-effort from the engine's view — any rejection is caught and
 * folded into the report, never propagated.
 *
 * `pullProgress` resolves to a {@link RemoteProgress} — structurally a
 * {@link Progress} plus the server `updatedAt` timestamp the newest-wins branch
 * needs — or `null`.
 */
export interface ProgressSource {
  /** Stable id, matched against `Book.sourceProviderId` when routing books. */
  readonly id: string;
  /** Human-friendly name shown next to the remote side of a conflict. */
  readonly displayName: string;
  /** Fetch the remote's current progress for `book`, or `null` if none. */
  pullProgress(book: Book): Promise<RemoteProgress | null>;
  /** Push a device-local position up to the remote. */
  pushProgress(book: Book, progress: Progress): Promise<void>;
}

/** A local progress value plus the device timestamp, when the store tracks one. */
export interface LocalProgress {
  progress: Progress;
  /** Epoch **seconds** of the local write, or `null` (our stores are untimed today). */
  updatedAt: number | null;
}

/**
 * The minimal local store surface for one lane: read the current {@link Progress}
 * for a book (or `null`), and write a reconciled winner back. The lane adapters
 * translate between {@link Progress} and each store's native position shape.
 */
export interface ProgressStore {
  get(bookId: string): Promise<LocalProgress | null>;
  put(bookId: string, progress: Progress): Promise<void>;
}
