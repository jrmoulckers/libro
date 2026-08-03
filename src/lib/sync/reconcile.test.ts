import { describe, it, expect } from 'vitest';
import type { Progress } from '../models';
import {
  reconcileProgress,
  isGenuineConflict,
  mergeRemoteWin,
  isProgressFinished,
  PROGRESS_TIE_EPSILON,
  RECENCY_TIE_SECONDS,
  type RemoteProgress,
} from './reconcile';

function local(fraction: number, finished = false): Progress {
  return { fraction, positionSeconds: fraction * 3600, locator: 'epubcfi(/6/4)', finished };
}

function remote(
  fraction: number,
  finished = false,
  updatedAt: number | null = null,
): RemoteProgress {
  return { fraction, positionSeconds: fraction * 3600, finished, updatedAt };
}

describe('reconcileProgress — presence table', () => {
  it('both absent → in-sync (nothing to do)', () => {
    expect(reconcileProgress(null, null, null, null)).toEqual({ kind: 'in-sync' });
  });

  it('local only → keep-local', () => {
    expect(reconcileProgress(local(0.3), null, null, null)).toEqual({ kind: 'keep-local' });
  });

  it('remote only → adopt remote', () => {
    const d = reconcileProgress(null, null, remote(0.4), null);
    expect(d.kind).toBe('use-remote');
    if (d.kind === 'use-remote') expect(d.progress.fraction).toBeCloseTo(0.4);
  });
});

describe('reconcileProgress — finished is sticky', () => {
  it('remote finished, local ahead but unfinished → remote wins finished, local locator preserved', () => {
    const d = reconcileProgress(local(0.7), null, remote(0.5, true), null);
    expect(d.kind).toBe('use-remote');
    if (d.kind === 'use-remote') {
      expect(d.progress.finished).toBe(true);
      expect(d.progress.fraction).toBeCloseTo(1);
      expect(d.progress.locator).toBe('epubcfi(/6/4)');
    }
  });

  it('local finished, remote in-progress → keep-local (never un-finish)', () => {
    expect(reconcileProgress(local(1, true), null, remote(0.3), null)).toEqual({
      kind: 'keep-local',
    });
  });

  it('both finished → in-sync', () => {
    expect(reconcileProgress(local(1, true), null, remote(1, true, 10), null)).toEqual({
      kind: 'in-sync',
    });
  });
});

describe('reconcileProgress — furthest-position fallback (no timestamps)', () => {
  it('remote further → use-remote', () => {
    const d = reconcileProgress(local(0.2), null, remote(0.6), null);
    expect(d.kind).toBe('use-remote');
    if (d.kind === 'use-remote') expect(d.progress.fraction).toBeCloseTo(0.6);
  });

  it('local further → keep-local', () => {
    expect(reconcileProgress(local(0.6), null, remote(0.2), null)).toEqual({ kind: 'keep-local' });
  });

  it('tiny delta within epsilon → in-sync (no thrash)', () => {
    expect(
      reconcileProgress(local(0.5), null, remote(0.5 + PROGRESS_TIE_EPSILON / 2), null),
    ).toEqual({ kind: 'in-sync' });
  });
});

describe('reconcileProgress — newest-wins branch', () => {
  it('remote newer wins even when behind (deliberate rewind)', () => {
    const d = reconcileProgress(local(0.6), 1000, remote(0.2, false, 2000), 2000);
    expect(d.kind).toBe('use-remote');
    if (d.kind === 'use-remote') expect(d.progress.fraction).toBeCloseTo(0.2);
  });

  it('local newer → keep-local even when behind', () => {
    expect(reconcileProgress(local(0.3), 2000, remote(0.9, false, 1000), 1000)).toEqual({
      kind: 'keep-local',
    });
  });

  it('timestamps within RECENCY_TIE_SECONDS → fall back to furthest-position', () => {
    const d = reconcileProgress(
      local(0.2),
      1000,
      remote(0.8, false, 1000 + RECENCY_TIE_SECONDS),
      1000 + RECENCY_TIE_SECONDS,
    );
    expect(d.kind).toBe('use-remote');
    if (d.kind === 'use-remote') expect(d.progress.fraction).toBeCloseTo(0.8);
  });
});

describe('reconcileProgress — policy: manual vs auto', () => {
  const l = local(0.2);
  const r = remote(0.7); // divergent, neither finished, no timestamps

  it('manual surfaces the ambiguous pair as a conflict', () => {
    expect(reconcileProgress(l, null, r, null, 'manual')).toEqual({ kind: 'conflict' });
  });

  it('auto resolves the same pair deterministically (furthest-position)', () => {
    const d = reconcileProgress(l, null, r, null, 'auto');
    expect(d.kind).toBe('use-remote');
    if (d.kind === 'use-remote') expect(d.progress.fraction).toBeCloseTo(0.7);
  });

  it('manual never surfaces a clear winner (one side finished)', () => {
    expect(reconcileProgress(l, null, remote(0.7, true), null, 'manual').kind).toBe('use-remote');
  });

  it('manual never surfaces a confidently-ordered pair (timestamps apart)', () => {
    expect(reconcileProgress(l, 1000, remote(0.7, false, 5000), 5000, 'manual').kind).toBe(
      'use-remote',
    );
  });
});

describe('isGenuineConflict', () => {
  it('true only for divergent, unfinished, unorderable pairs', () => {
    expect(isGenuineConflict(local(0.2), null, remote(0.7), null)).toBe(true);
  });

  it('false when a side is finished', () => {
    expect(isGenuineConflict(local(0.2), null, remote(0.7, true), null)).toBe(false);
    expect(isGenuineConflict(local(1, true), null, remote(0.7), null)).toBe(false);
  });

  it('false when within the tie window', () => {
    expect(isGenuineConflict(local(0.5), null, remote(0.505), null)).toBe(false);
  });

  it('false when timestamps confidently order the pair', () => {
    expect(isGenuineConflict(local(0.2), 1000, remote(0.7), 9000)).toBe(false);
  });

  it('agrees with reconcileProgress(manual) → conflict iff genuine', () => {
    const cases: Array<[Progress, number | null, RemoteProgress]> = [
      [local(0.2), null, remote(0.7)],
      [local(0.2), null, remote(0.7, true)],
      [local(0.5), null, remote(0.505)],
      [local(0.2), 1000, remote(0.7, false, 9000)],
    ];
    for (const [l, lt, r] of cases) {
      const genuine = isGenuineConflict(l, lt, r, r.updatedAt);
      const decision = reconcileProgress(l, lt, r, r.updatedAt, 'manual');
      expect(decision.kind === 'conflict').toBe(genuine);
    }
  });
});

describe('mergeRemoteWin', () => {
  it('preserves local locator and falls back to local seconds when remote lacks them', () => {
    const merged = mergeRemoteWin({ fraction: 0.5, finished: false }, local(0.1));
    expect(merged.locator).toBe('epubcfi(/6/4)');
    expect(merged.positionSeconds).toBeCloseTo(0.1 * 3600);
  });

  it('clamps a finished remote to fraction 1', () => {
    const merged = mergeRemoteWin({ fraction: 0.4, finished: true }, null);
    expect(merged.finished).toBe(true);
    expect(merged.fraction).toBe(1);
  });
});

describe('isProgressFinished', () => {
  it('true for explicit flag or ~complete fraction', () => {
    expect(isProgressFinished({ fraction: 0.1, finished: true })).toBe(true);
    expect(isProgressFinished({ fraction: 0.995, finished: false })).toBe(true);
    expect(isProgressFinished({ fraction: 0.5, finished: false })).toBe(false);
  });
});
