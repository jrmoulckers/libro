//! Inbound (pull-down) progress sync + reconciliation — the counterpart to the
//! two **outward** syncs ([`crate::sync`] reading→Hardcover and
//! [`crate::listening_sync`] listening→Audiobookshelf).
//!
//! When the library loads (or on an explicit refresh), Libro can pull each
//! remote's *current* progress **down**, reconcile it against the on-device
//! [`crate::config::ReadingStore`] / [`crate::config::ListeningStore`] with a
//! clear conflict policy, and write the winner back locally so a book
//! read/listened on another device resumes at the right place here.
//!
//! ## Why a sibling module (not folded into `sync`/`listening_sync`)
//!
//! The outward engines each own **one** tracker surface, throttle metric, and
//! outcome type, and they *push* a single item's state. Reconciliation is a
//! different shape: it *reads* from a remote, *compares* two positions under a
//! documented policy, and *writes the local store*. It also spans **both** lanes
//! (audio vs. reading) behind one [`ProgressSource`] trait and one
//! [`reconcile`] policy. Keeping it a sibling avoids overloading either outward
//! engine with an inbound/merge responsibility they don't have.
//!
//! ## Lanes never cross
//!
//! An audiobook (seconds / [`ListeningStore`](crate::config::ListeningStore)) is
//! reconciled against Audiobookshelf; an ebook (CFI /
//! [`ReadingStore`](crate::config::ReadingStore)) against Hardcover. The apply
//! pass picks the lane per book; [`reconcile`] itself is lane-agnostic.
//!
//! ## Conflict policy (precise)
//!
//! [`reconcile`] decides a winner as follows:
//! 1. **`finished` is sticky.** Once an item is finished on *either* side it
//!    stays finished (finishing is not undone by a stale in-progress record).
//! 2. **Newest-wins by `updated_at`** (last-write-wins) when *both* sides carry a
//!    timestamp and they differ by more than [`RECENCY_TIE_SECONDS`].
//! 3. **Furthest-position-wins** as the fallback when a timestamp is missing or
//!    the two are within the recency tie window — the larger `fraction` wins.
//! 4. **Tie/threshold, no thrash.** Fractions within [`PROGRESS_TIE_EPSILON`] are
//!    treated as already in sync, so tiny deltas never trigger a write.
//!
//! Note on timestamps: the local [`crate::models::Progress`] model does not yet
//! carry a per-write timestamp, so the public [`reconcile`] passes `None` for the
//! local time and unfinished conflicts fall back to furthest-position-wins today.
//! The newest-wins branch is implemented (and unit-tested) via [`reconcile_with`]
//! so it is ready the moment a local timestamp lands (the Signal-style
//! per-device sync model anticipated on [`Progress`]).
//!
//! ## Guarantees (same posture as the outward syncs)
//!
//! * **Opt-in.** Gated on a per-provider `pull_progress` flag (default `false`).
//! * **Failure isolation.** The local store is the source of truth; every
//!   remote/network error is captured into a [`ReconcileOutcome`] and *never*
//!   propagated, so a failed pull can't break library load or the reader/player.
//! * **No feedback loop.** The reconciled write goes straight to the store,
//!   bypassing the outward engines, so a pulled-down value is not immediately
//!   pushed back out.

use async_trait::async_trait;

use crate::models::{Book, Progress};
use crate::providers::ProviderResult;

/// Fractions within this absolute delta are considered "the same position", so a
/// negligible difference never causes a store write (no-thrash).
pub const PROGRESS_TIE_EPSILON: f32 = 0.01;

/// When both sides carry an `updated_at`, timestamps closer than this (seconds)
/// are treated as a tie and fall back to furthest-position-wins.
pub const RECENCY_TIE_SECONDS: i64 = 2;

/// A normalized snapshot of a book's progress on a *remote* service, produced by
/// a [`ProgressSource`] from its own API shape.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteProgress {
    /// Fractional completion in `0.0..=1.0` (coarse for status-only sources).
    pub fraction: f32,
    /// Position in seconds when the source tracks one (audio); `None` otherwise.
    pub position_seconds: Option<f64>,
    /// Whether the remote considers the item finished.
    pub finished: bool,
    /// Server last-update time (epoch seconds) when exposed; `None` otherwise.
    /// Drives the newest-wins branch of [`reconcile`].
    pub updated_at: Option<i64>,
}

impl RemoteProgress {
    /// Whether this remote record counts as finished (explicit flag or ~complete).
    pub fn is_finished(&self) -> bool {
        self.finished || self.fraction >= 0.99
    }
}

/// The minimal **read** surface the reconciliation pass needs from a remote.
///
/// Implemented by real connectors (`AudiobookshelfProvider`, `HardcoverProvider`)
/// and by fakes in tests. Best-effort from the engine's perspective: any `Err`
/// is caught and folded into a [`ReconcileOutcome`], never propagated.
#[async_trait]
pub trait ProgressSource: Send + Sync {
    /// Fetch the remote's current progress for `book`, or `None` when the remote
    /// has no record for it.
    async fn fetch_remote_progress(&self, book: &Book) -> ProviderResult<Option<RemoteProgress>>;
}

/// The minimal **write** surface the reconciliation pass needs from a local
/// store. Implemented by [`ReadingStore`](crate::config::ReadingStore) and
/// [`ListeningStore`](crate::config::ListeningStore), and by a fake in tests.
pub trait ProgressStoreLike: Send + Sync {
    /// Current local progress for `key` (the local `Book.id`), if any. Read
    /// errors are treated as "no local value".
    fn get_progress(&self, key: &str) -> Option<Progress>;
    /// Persist the reconciled progress for `key`. Errors surface as a `String`.
    fn put_progress(&self, key: &str, value: Progress) -> Result<(), String>;
}

/// The outcome of reconciling one book's local vs. remote progress.
#[derive(Debug, Clone, PartialEq)]
pub enum Reconciliation {
    /// Neither side has any progress.
    NoData,
    /// The two agree (within thresholds); nothing to write.
    AlreadyInSync,
    /// The local value is authoritative; leave the store untouched.
    LocalWins,
    /// The remote is authoritative; write this merged [`Progress`] to the store.
    RemoteWins(Progress),
}

/// Reconcile a local and a remote progress under the module's policy, with no
/// local timestamp available (see the module docs). Never fails.
pub fn reconcile(local: Option<&Progress>, remote: Option<&RemoteProgress>) -> Reconciliation {
    reconcile_with(local, None, remote)
}

/// Like [`reconcile`] but with an explicit local `updated_at` (epoch seconds),
/// enabling the newest-wins branch. Kept separate so the recency policy is
/// unit-testable and ready for a future timestamped local store.
pub fn reconcile_with(
    local: Option<&Progress>,
    local_updated_at: Option<i64>,
    remote: Option<&RemoteProgress>,
) -> Reconciliation {
    match (local, remote) {
        (None, None) => Reconciliation::NoData,
        // Nothing remote to pull — keep whatever is local.
        (Some(_), None) => Reconciliation::LocalWins,
        // No local value yet — adopt the remote outright.
        (None, Some(r)) => Reconciliation::RemoteWins(remote_to_progress(r, None)),
        (Some(l), Some(r)) => reconcile_both(l, local_updated_at, r),
    }
}

fn local_is_finished(p: &Progress) -> bool {
    p.finished || p.fraction >= 0.99
}

fn reconcile_both(local: &Progress, local_updated_at: Option<i64>, remote: &RemoteProgress) -> Reconciliation {
    let l_fin = local_is_finished(local);
    let r_fin = remote.is_finished();

    // 1. finished is sticky.
    match (l_fin, r_fin) {
        (true, true) => return Reconciliation::AlreadyInSync,
        (true, false) => return Reconciliation::LocalWins, // don't un-finish locally
        (false, true) => return Reconciliation::RemoteWins(remote_to_progress(remote, Some(local))),
        (false, false) => {}
    }

    let in_sync = (local.fraction - remote.fraction).abs() <= PROGRESS_TIE_EPSILON;

    // 2. newest-wins when both timestamps are present and meaningfully apart.
    if let (Some(lt), Some(rt)) = (local_updated_at, remote.updated_at) {
        if (lt - rt).abs() > RECENCY_TIE_SECONDS {
            if in_sync {
                return Reconciliation::AlreadyInSync;
            }
            return if rt > lt {
                Reconciliation::RemoteWins(remote_to_progress(remote, Some(local)))
            } else {
                Reconciliation::LocalWins
            };
        }
    }

    // 3. furthest-position-wins fallback (with the no-thrash tie window).
    if remote.fraction - local.fraction > PROGRESS_TIE_EPSILON {
        Reconciliation::RemoteWins(remote_to_progress(remote, Some(local)))
    } else if local.fraction - remote.fraction > PROGRESS_TIE_EPSILON {
        Reconciliation::LocalWins
    } else {
        Reconciliation::AlreadyInSync
    }
}

/// Build the [`Progress`] to store when the remote wins, merging in local fields
/// the remote doesn't carry: remote sources have no EPUB CFI, and status-only
/// sources have no seconds — so keep the local `locator`/`position_seconds` as a
/// fallback rather than dropping the reader's precise resume point.
fn remote_to_progress(remote: &RemoteProgress, local: Option<&Progress>) -> Progress {
    let finished = remote.is_finished();
    Progress {
        fraction: if finished { 1.0 } else { remote.fraction },
        position_seconds: remote
            .position_seconds
            .or_else(|| local.and_then(|l| l.position_seconds)),
        locator: local.and_then(|l| l.locator.clone()),
        finished,
    }
}

/// The conflict-resolution policy for the inbound reconciliation pass.
///
/// `Auto` (the default) keeps the historical behavior: every book is resolved by
/// [`reconcile`] (last-write-wins / furthest-position) and written straight to
/// the store. `Manual` diverts only *genuine* conflicts (see
/// [`is_genuine_conflict`]) to a pending set for the user to resolve; clear
/// winners still auto-apply, so switching to `Manual` never regresses the
/// unambiguous cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictResolution {
    /// Auto last-write-wins / furthest-position (unchanged behavior).
    #[default]
    Auto,
    /// Surface genuine conflicts for manual resolution; clear winners auto-apply.
    Manual,
}

impl ConflictResolution {
    /// Whether ambiguous conflicts should be surfaced rather than auto-resolved.
    pub fn is_manual(self) -> bool {
        matches!(self, ConflictResolution::Manual)
    }
}

/// Detect a **genuine, unorderable** conflict between a local and a remote
/// progress — the only case a `Manual` policy surfaces to the user.
///
/// Returns `true` only when *all* of the following hold:
/// * **Both sides carry meaningful, divergent progress** — the fractions differ
///   by more than [`PROGRESS_TIE_EPSILON`] (tiny deltas are "in sync", never a
///   conflict).
/// * **Neither side is finished** — the finished-sticky rule always resolves
///   deterministically, so a finished record is never a conflict.
/// * **Timestamps cannot confidently order them** — either side is missing an
///   `updated_at`, or the two are within [`RECENCY_TIE_SECONDS`]. When both are
///   present and meaningfully apart, newest-wins decides and there is no
///   conflict.
///
/// A clear winner (one side newer, one side finished, or the two within the tie
/// window) returns `false` and still auto-resolves exactly as before. This is a
/// pure classifier with no side effects.
pub fn is_genuine_conflict(
    local: &Progress,
    local_updated_at: Option<i64>,
    remote: &RemoteProgress,
) -> bool {
    // Finished-sticky resolves deterministically — never a conflict.
    if local_is_finished(local) || remote.is_finished() {
        return false;
    }
    // Positions within the no-thrash window are "in sync" — never a conflict.
    if (local.fraction - remote.fraction).abs() <= PROGRESS_TIE_EPSILON {
        return false;
    }
    // Both timestamps present and meaningfully apart ⇒ newest-wins can decide.
    if let (Some(lt), Some(rt)) = (local_updated_at, remote.updated_at) {
        if (lt - rt).abs() > RECENCY_TIE_SECONDS {
            return false;
        }
    }
    // Divergent positions with no confident ordering ⇒ genuine conflict.
    true
}

/// Which local store lane a pending conflict belongs to (frontend-facing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictLane {
    /// Audiobook progress (seconds ↔ Audiobookshelf).
    Listening,
    /// Ebook reading progress (CFI ↔ Hardcover).
    Reading,
}

impl From<Lane> for ConflictLane {
    fn from(lane: Lane) -> Self {
        match lane {
            Lane::Audio => ConflictLane::Listening,
            Lane::Reading => ConflictLane::Reading,
        }
    }
}

/// One side (local or remote) of a pending conflict, for rendering the choice.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ConflictSide {
    /// Fractional completion in `0.0..=1.0`.
    pub fraction: f32,
    /// Position in seconds when tracked (audio); `None` otherwise.
    pub position_seconds: Option<f64>,
    /// Whether this side considers the item finished.
    pub finished: bool,
    /// Server last-update time (epoch seconds) when the side exposes one.
    pub updated_at: Option<i64>,
    /// Human label for the side ("This device" locally, the source name remotely).
    pub source: String,
}

/// A conflict awaiting manual resolution: enough to render both options and,
/// once chosen, write the correct [`Progress`] into the correct lane.
///
/// The two `*_progress` payloads are the concrete values each choice would
/// write; they are kept off the wire (`serde(skip)`) since the UI only needs the
/// display [`ConflictSide`]s — the resolver reads them from the pending store.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProgressConflict {
    pub book_id: String,
    pub title: String,
    pub lane: ConflictLane,
    pub local: ConflictSide,
    pub remote: ConflictSide,
    #[serde(skip)]
    pub local_progress: Progress,
    #[serde(skip)]
    pub remote_progress: Progress,
}

/// The user's choice when resolving a [`ProgressConflict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    /// Keep this device's local progress.
    KeepLocal,
    /// Adopt the remote's progress.
    UseRemote,
    /// Keep whichever side is furthest along (largest fraction).
    KeepFurthest,
}

impl ProgressConflict {
    /// The concrete [`Progress`] a given [`ConflictChoice`] resolves to.
    pub fn resolved(&self, choice: ConflictChoice) -> Progress {
        match choice {
            ConflictChoice::KeepLocal => self.local_progress.clone(),
            ConflictChoice::UseRemote => self.remote_progress.clone(),
            ConflictChoice::KeepFurthest => {
                if self.remote_progress.fraction > self.local_progress.fraction {
                    self.remote_progress.clone()
                } else {
                    self.local_progress.clone()
                }
            }
        }
    }
}

/// Build a [`ProgressConflict`] from a book, its lane, and the divergent
/// local/remote records. The remote payload is merged via [`remote_to_progress`]
/// (preserving local fields the remote doesn't carry, e.g. an EPUB CFI).
fn build_conflict(
    book: &Book,
    lane: Lane,
    local: &Progress,
    remote: &RemoteProgress,
) -> ProgressConflict {
    let source_name = match lane {
        Lane::Audio => "Audiobookshelf",
        Lane::Reading => "Hardcover",
    };
    ProgressConflict {
        book_id: book.id.clone(),
        title: book.title.clone(),
        lane: lane.into(),
        local: ConflictSide {
            fraction: local.fraction,
            position_seconds: local.position_seconds,
            finished: local.finished,
            updated_at: None,
            source: "This device".to_string(),
        },
        remote: ConflictSide {
            fraction: remote.fraction,
            position_seconds: remote.position_seconds,
            finished: remote.finished,
            updated_at: remote.updated_at,
            source: source_name.to_string(),
        },
        local_progress: local.clone(),
        remote_progress: remote_to_progress(remote, Some(local)),
    }
}

/// What the apply pass did for one book — returned (never thrown) so the caller
/// can tally it and move on.
#[derive(Debug, Clone, PartialEq)]
pub enum ReconcileOutcome {
    /// Inbound pull is off for this provider (opt-in flag false).
    Disabled,
    /// The remote had no record for this book (or returned nothing).
    NoRemote,
    /// Local and remote already agree; store left untouched.
    AlreadyInSync,
    /// Local value kept; store left untouched.
    KeptLocal,
    /// Remote won; the merged progress was written to the local store.
    PulledDown(Progress),
    /// Manual mode: a genuine conflict was recorded as pending; store untouched.
    Conflict,
    /// A remote/store error occurred and was swallowed; message is for logs.
    Failed(String),
}

/// Reconcile one book against a remote source and apply the result to a local
/// store. Best-effort: any source/store error is captured in the returned
/// [`ReconcileOutcome`], and the local store is only ever written on `RemoteWins`.
///
/// `enabled` is the provider's opt-in `pull_progress` flag.
pub async fn reconcile_book_into_store<S: ProgressSource + ?Sized>(
    enabled: bool,
    source: &S,
    store: &dyn ProgressStoreLike,
    book: &Book,
) -> ReconcileOutcome {
    if !enabled {
        return ReconcileOutcome::Disabled;
    }

    let remote = match source.fetch_remote_progress(book).await {
        Ok(Some(r)) => r,
        Ok(None) => return ReconcileOutcome::NoRemote,
        Err(e) => return ReconcileOutcome::Failed(format!("fetch_remote_progress failed: {e}")),
    };

    let local = store.get_progress(&book.id);
    match reconcile(local.as_ref(), Some(&remote)) {
        Reconciliation::RemoteWins(p) => match store.put_progress(&book.id, p.clone()) {
            Ok(()) => ReconcileOutcome::PulledDown(p),
            Err(e) => ReconcileOutcome::Failed(format!("store write failed: {e}")),
        },
        Reconciliation::LocalWins => ReconcileOutcome::KeptLocal,
        Reconciliation::AlreadyInSync => ReconcileOutcome::AlreadyInSync,
        Reconciliation::NoData => ReconcileOutcome::NoRemote,
    }
}

/// A small tally of an apply pass over many books, surfaced to the caller/UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ReconcileReport {
    pub pulled_down: u32,
    pub kept_local: u32,
    pub already_in_sync: u32,
    pub no_remote: u32,
    pub disabled: u32,
    /// Genuine conflicts recorded as pending in manual mode (store untouched).
    pub conflicts: u32,
    pub failed: u32,
}

impl ReconcileReport {
    /// Fold one book's outcome into the running tally.
    pub fn record(&mut self, outcome: &ReconcileOutcome) {
        match outcome {
            ReconcileOutcome::PulledDown(_) => self.pulled_down += 1,
            ReconcileOutcome::KeptLocal => self.kept_local += 1,
            ReconcileOutcome::AlreadyInSync => self.already_in_sync += 1,
            ReconcileOutcome::NoRemote => self.no_remote += 1,
            ReconcileOutcome::Disabled => self.disabled += 1,
            ReconcileOutcome::Conflict => self.conflicts += 1,
            ReconcileOutcome::Failed(_) => self.failed += 1,
        }
    }
}

/// One lane's wiring for the batch reconciliation pass: a remote read surface, a
/// local write surface, and whether the user opted this lane in.
pub struct SyncLane<'a> {
    pub enabled: bool,
    pub source: &'a (dyn ProgressSource + 'a),
    pub store: &'a (dyn ProgressStoreLike + 'a),
}

/// Which reconciliation lane a book belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Audiobook progress (seconds) ↔ Audiobookshelf.
    Audio,
    /// Ebook reading progress (CFI) ↔ Hardcover.
    Reading,
}

/// Decide a book's lane, or `None` if it isn't reconcilable.
///
/// Lanes never cross: an ABS-sourced audiobook is Audio; any other ebook is
/// Reading (a Hardcover-sourced book is skipped — we don't reconcile Hardcover's
/// own shelf list against itself).
pub fn lane_for(book: &Book) -> Option<Lane> {
    use crate::models::MediaType;
    let abs = crate::providers::audiobookshelf::AudiobookshelfProvider::ID;
    let hc = crate::providers::hardcover::HardcoverProvider::ID;
    if book.source_provider_id == abs && book.media_type == MediaType::Audiobook {
        Some(Lane::Audio)
    } else if book.media_type == MediaType::Ebook && book.source_provider_id != hc {
        Some(Lane::Reading)
    } else {
        None
    }
}

/// Batch inbound reconciliation over a loaded catalog.
///
/// Runs the remote fetches with **bounded concurrency** (`concurrency`, like the
/// metadata-enrichment pass) so a large library never bursts the APIs, then
/// applies the winners to the local stores **sequentially** (the file stores
/// share a temp path, so serialized writes avoid a race). Best-effort throughout:
/// every fetch/store error is swallowed into the returned [`ReconcileReport`];
/// this never fails. Books with no matching/enabled lane are skipped.
pub async fn reconcile_catalog(
    books: &[Book],
    audio: Option<&SyncLane<'_>>,
    reading: Option<&SyncLane<'_>>,
    concurrency: usize,
) -> ReconcileReport {
    reconcile_catalog_with_policy(books, audio, reading, concurrency, ConflictResolution::Auto)
        .await
        .0
}

/// Policy-aware batch reconciliation. Identical to [`reconcile_catalog`] under
/// [`ConflictResolution::Auto`] (and returns an empty conflict list). Under
/// [`ConflictResolution::Manual`], a *genuine* conflict (see
/// [`is_genuine_conflict`]) is **recorded as a pending [`ProgressConflict`]
/// instead of written**; clear winners still auto-apply exactly as in auto mode.
///
/// Returns the [`ReconcileReport`] tally plus the pending conflicts to surface.
pub async fn reconcile_catalog_with_policy(
    books: &[Book],
    audio: Option<&SyncLane<'_>>,
    reading: Option<&SyncLane<'_>>,
    concurrency: usize,
    policy: ConflictResolution,
) -> (ReconcileReport, Vec<ProgressConflict>) {
    use futures::stream::{self, StreamExt};

    let lane_ref = |lane: Lane| match lane {
        Lane::Audio => audio,
        Lane::Reading => reading,
    };

    // Pre-compute the enabled work list (owned) so the async stream below does
    // not carry a borrowing closure (which trips higher-ranked-lifetime
    // inference when this runs inside a Tauri command future).
    let work: Vec<(usize, Lane)> = books
        .iter()
        .enumerate()
        .filter_map(|(i, b)| lane_for(b).map(|l| (i, l)))
        .filter(|(_, l)| lane_ref(*l).map(|ln| ln.enabled).unwrap_or(false))
        .collect();

    // Phase 1 — bounded-concurrency remote fetches (network-bound, read-only).
    let fetched: Vec<(usize, Lane, Result<Option<RemoteProgress>, String>)> = stream::iter(work)
        .map(|(i, lane)| async move {
            let src = lane_ref(lane).expect("lane present").source;
            let res = src
                .fetch_remote_progress(&books[i])
                .await
                .map_err(|e| e.to_string());
            (i, lane, res)
        })
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await;

    // Phase 2 — sequential reconcile + apply (serialized store writes).
    let mut report = ReconcileReport::default();
    let mut conflicts: Vec<ProgressConflict> = Vec::new();
    for (i, lane, res) in fetched {
        let store = lane_ref(lane).expect("lane present").store;
        let outcome = match res {
            Err(e) => ReconcileOutcome::Failed(e),
            Ok(None) => ReconcileOutcome::NoRemote,
            Ok(Some(remote)) => {
                let local = store.get_progress(&books[i].id);
                // Manual mode: divert a genuine conflict to the pending set
                // (no store write); clear winners fall through to auto-apply.
                if policy.is_manual() {
                    if let Some(l) = local.as_ref() {
                        if is_genuine_conflict(l, None, &remote) {
                            conflicts.push(build_conflict(&books[i], lane, l, &remote));
                            report.record(&ReconcileOutcome::Conflict);
                            continue;
                        }
                    }
                }
                match reconcile(local.as_ref(), Some(&remote)) {
                    Reconciliation::RemoteWins(p) => {
                        match store.put_progress(&books[i].id, p.clone()) {
                            Ok(()) => ReconcileOutcome::PulledDown(p),
                            Err(e) => ReconcileOutcome::Failed(format!("store write failed: {e}")),
                        }
                    }
                    Reconciliation::LocalWins => ReconcileOutcome::KeptLocal,
                    Reconciliation::AlreadyInSync => ReconcileOutcome::AlreadyInSync,
                    Reconciliation::NoData => ReconcileOutcome::NoRemote,
                }
            }
        };
        report.record(&outcome);
    }
    (report, conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaType;
    use crate::providers::ProviderError;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn local(fraction: f32, finished: bool) -> Progress {
        Progress {
            fraction,
            position_seconds: Some((fraction * 3600.0) as f64),
            locator: Some("epubcfi(/6/4)".into()),
            finished,
        }
    }

    fn remote(fraction: f32, finished: bool, updated_at: Option<i64>) -> RemoteProgress {
        RemoteProgress {
            fraction,
            position_seconds: Some((fraction * 3600.0) as f64),
            finished,
            updated_at,
        }
    }

    // ---- reconcile() truth table -----------------------------------------

    #[test]
    fn no_data_when_both_absent() {
        assert_eq!(reconcile(None, None), Reconciliation::NoData);
    }

    #[test]
    fn local_only_keeps_local() {
        assert_eq!(reconcile(Some(&local(0.3, false)), None), Reconciliation::LocalWins);
    }

    #[test]
    fn remote_only_is_adopted() {
        let r = remote(0.4, false, None);
        match reconcile(None, Some(&r)) {
            Reconciliation::RemoteWins(p) => assert!((p.fraction - 0.4).abs() < 1e-6),
            other => panic!("expected RemoteWins, got {other:?}"),
        }
    }

    #[test]
    fn furthest_position_wins_when_no_timestamps() {
        // remote further along → pull down.
        match reconcile(Some(&local(0.2, false)), Some(&remote(0.6, false, None))) {
            Reconciliation::RemoteWins(p) => assert!((p.fraction - 0.6).abs() < 1e-6),
            other => panic!("expected RemoteWins, got {other:?}"),
        }
        // local further along → keep local.
        assert_eq!(
            reconcile(Some(&local(0.6, false)), Some(&remote(0.2, false, None))),
            Reconciliation::LocalWins
        );
    }

    #[test]
    fn tiny_delta_is_already_in_sync_no_thrash() {
        // 0.500 vs 0.505 is within PROGRESS_TIE_EPSILON → no write.
        assert_eq!(
            reconcile(Some(&local(0.500, false)), Some(&remote(0.505, false, None))),
            Reconciliation::AlreadyInSync
        );
    }

    #[test]
    fn finished_is_sticky_remote_finished_pulls_finished_down() {
        // remote finished, local ahead in fraction but not finished → finished wins.
        match reconcile(Some(&local(0.7, false)), Some(&remote(0.5, true, None))) {
            Reconciliation::RemoteWins(p) => {
                assert!(p.finished);
                assert!((p.fraction - 1.0).abs() < 1e-6);
                // local locator is preserved through the merge.
                assert_eq!(p.locator.as_deref(), Some("epubcfi(/6/4)"));
            }
            other => panic!("expected RemoteWins(finished), got {other:?}"),
        }
    }

    #[test]
    fn finished_is_sticky_local_finished_stays_finished() {
        // local finished, remote merely in-progress → keep local (don't un-finish).
        assert_eq!(
            reconcile(Some(&local(1.0, true)), Some(&remote(0.3, false, None))),
            Reconciliation::LocalWins
        );
    }

    #[test]
    fn both_finished_is_already_in_sync() {
        assert_eq!(
            reconcile(Some(&local(1.0, true)), Some(&remote(1.0, true, Some(10)))),
            Reconciliation::AlreadyInSync
        );
    }

    // ---- newest-wins branch (via reconcile_with) -------------------------

    #[test]
    fn newest_wins_remote_newer_pulls_down_even_if_behind() {
        // Last-write-wins: remote is newer, so it wins although its fraction is
        // lower (a deliberate rewind on another device).
        let r = remote(0.2, false, Some(2_000));
        match reconcile_with(Some(&local(0.6, false)), Some(1_000), Some(&r)) {
            Reconciliation::RemoteWins(p) => assert!((p.fraction - 0.2).abs() < 1e-6),
            other => panic!("expected RemoteWins, got {other:?}"),
        }
    }

    #[test]
    fn newest_wins_local_newer_keeps_local() {
        let r = remote(0.9, false, Some(1_000));
        assert_eq!(
            reconcile_with(Some(&local(0.3, false)), Some(2_000), Some(&r)),
            Reconciliation::LocalWins
        );
    }

    #[test]
    fn recency_tie_falls_back_to_furthest_position() {
        // Timestamps within RECENCY_TIE_SECONDS → ignore recency, use position.
        let r = remote(0.8, false, Some(1_001));
        match reconcile_with(Some(&local(0.2, false)), Some(1_000), Some(&r)) {
            Reconciliation::RemoteWins(p) => assert!((p.fraction - 0.8).abs() < 1e-6),
            other => panic!("expected RemoteWins via furthest-position, got {other:?}"),
        }
    }

    // ---- apply pass (reconcile_book_into_store) --------------------------

    /// In-memory store implementing the write surface, for the apply-pass tests.
    #[derive(Default)]
    struct FakeStore {
        map: Mutex<HashMap<String, Progress>>,
        fail_writes: bool,
    }
    impl ProgressStoreLike for FakeStore {
        fn get_progress(&self, key: &str) -> Option<Progress> {
            self.map.lock().unwrap().get(key).cloned()
        }
        fn put_progress(&self, key: &str, value: Progress) -> Result<(), String> {
            if self.fail_writes {
                return Err("disk full".into());
            }
            self.map.lock().unwrap().insert(key.to_string(), value);
            Ok(())
        }
    }

    struct FakeSource {
        remote: Option<RemoteProgress>,
        fail: bool,
    }
    #[async_trait]
    impl ProgressSource for FakeSource {
        async fn fetch_remote_progress(
            &self,
            _book: &Book,
        ) -> ProviderResult<Option<RemoteProgress>> {
            if self.fail {
                return Err(ProviderError::Network("boom".into()));
            }
            Ok(self.remote.clone())
        }
    }

    fn book(id: &str) -> Book {
        Book::new(id, "T", MediaType::Audiobook, "audiobookshelf")
    }

    #[tokio::test]
    async fn disabled_flag_never_touches_source_or_store() {
        let src = FakeSource { remote: Some(remote(0.9, false, None)), fail: false };
        let store = FakeStore::default();
        store.map.lock().unwrap().insert("b1".into(), local(0.1, false));

        let out = reconcile_book_into_store(false, &src, &store, &book("b1")).await;
        assert_eq!(out, ReconcileOutcome::Disabled);
        // Store untouched.
        assert!((store.get_progress("b1").unwrap().fraction - 0.1).abs() < 1e-6);
    }

    #[tokio::test]
    async fn remote_wins_is_written_to_the_store() {
        let src = FakeSource { remote: Some(remote(0.8, false, None)), fail: false };
        let store = FakeStore::default();
        store.map.lock().unwrap().insert("b1".into(), local(0.2, false));

        let out = reconcile_book_into_store(true, &src, &store, &book("b1")).await;
        assert!(matches!(out, ReconcileOutcome::PulledDown(_)));
        assert!((store.get_progress("b1").unwrap().fraction - 0.8).abs() < 1e-6);
    }

    #[tokio::test]
    async fn local_wins_leaves_store_untouched() {
        let src = FakeSource { remote: Some(remote(0.2, false, None)), fail: false };
        let store = FakeStore::default();
        store.map.lock().unwrap().insert("b1".into(), local(0.9, false));

        let out = reconcile_book_into_store(true, &src, &store, &book("b1")).await;
        assert_eq!(out, ReconcileOutcome::KeptLocal);
        assert!((store.get_progress("b1").unwrap().fraction - 0.9).abs() < 1e-6);
    }

    #[tokio::test]
    async fn no_remote_record_is_a_noop() {
        let src = FakeSource { remote: None, fail: false };
        let store = FakeStore::default();
        let out = reconcile_book_into_store(true, &src, &store, &book("b1")).await;
        assert_eq!(out, ReconcileOutcome::NoRemote);
    }

    #[tokio::test]
    async fn erroring_source_is_swallowed_and_store_untouched() {
        let src = FakeSource { remote: None, fail: true };
        let store = FakeStore::default();
        store.map.lock().unwrap().insert("b1".into(), local(0.3, false));

        let out = reconcile_book_into_store(true, &src, &store, &book("b1")).await;
        assert!(matches!(out, ReconcileOutcome::Failed(_)));
        // Local store is the source of truth — untouched by a failed pull.
        assert!((store.get_progress("b1").unwrap().fraction - 0.3).abs() < 1e-6);
    }

    #[tokio::test]
    async fn store_write_failure_is_reported_not_panicked() {
        let src = FakeSource { remote: Some(remote(0.8, false, None)), fail: false };
        let store = FakeStore { fail_writes: true, ..Default::default() };
        let out = reconcile_book_into_store(true, &src, &store, &book("b1")).await;
        assert!(matches!(out, ReconcileOutcome::Failed(_)));
    }

    #[test]
    fn report_tallies_outcomes() {
        let mut r = ReconcileReport::default();
        r.record(&ReconcileOutcome::PulledDown(local(1.0, true)));
        r.record(&ReconcileOutcome::KeptLocal);
        r.record(&ReconcileOutcome::AlreadyInSync);
        r.record(&ReconcileOutcome::NoRemote);
        r.record(&ReconcileOutcome::Disabled);
        r.record(&ReconcileOutcome::Conflict);
        r.record(&ReconcileOutcome::Failed("x".into()));
        assert_eq!(
            r,
            ReconcileReport {
                pulled_down: 1,
                kept_local: 1,
                already_in_sync: 1,
                no_remote: 1,
                disabled: 1,
                conflicts: 1,
                failed: 1,
            }
        );
    }

    // ---- lane routing + batch pass ---------------------------------------

    fn ebook(id: &str, provider: &str) -> Book {
        Book::new(id, "E", MediaType::Ebook, provider)
    }

    #[test]
    fn lane_for_routes_by_media_and_source() {
        assert_eq!(lane_for(&book("a1")), Some(Lane::Audio)); // ABS audiobook
        assert_eq!(lane_for(&ebook("e1", "localfiles")), Some(Lane::Reading));
        assert_eq!(lane_for(&ebook("e2", "opds")), Some(Lane::Reading));
        // Hardcover's own shelf list is not reconciled against itself.
        assert_eq!(lane_for(&ebook("e3", "hardcover")), None);
        // A non-ABS audiobook has no lane.
        assert_eq!(
            lane_for(&Book::new("a2", "A", MediaType::Audiobook, "localfiles")),
            None
        );
    }

    #[tokio::test]
    async fn reconcile_catalog_routes_lanes_and_applies_winners() {
        // Audio lane: remote further along → pulled down into the listening store.
        let audio_src = FakeSource { remote: Some(remote(0.7, false, None)), fail: false };
        let audio_store = FakeStore::default();
        audio_store.map.lock().unwrap().insert("a1".into(), local(0.1, false));
        let audio_lane = SyncLane { enabled: true, source: &audio_src, store: &audio_store };

        // Reading lane: remote finished → pulled down into the reading store.
        let reading_src = FakeSource { remote: Some(remote(1.0, true, None)), fail: false };
        let reading_store = FakeStore::default();
        let reading_lane = SyncLane { enabled: true, source: &reading_src, store: &reading_store };

        let books = vec![
            book("a1"),               // Audio
            ebook("e1", "localfiles"),// Reading
            ebook("e3", "hardcover"), // skipped (no lane)
        ];

        let report =
            reconcile_catalog(&books, Some(&audio_lane), Some(&reading_lane), 4).await;

        assert_eq!(report.pulled_down, 2, "both audio and reading pulled down");
        // Audio store got the remote position…
        assert!((audio_store.get_progress("a1").unwrap().fraction - 0.7).abs() < 1e-6);
        // …and the reading store got a finished record it didn't have before.
        assert!(reading_store.get_progress("e1").unwrap().finished);
    }

    #[tokio::test]
    async fn reconcile_catalog_skips_disabled_lane_and_isolates_failures() {
        // Audio lane disabled → its book is never fetched or written.
        let audio_src = FakeSource { remote: Some(remote(0.9, false, None)), fail: false };
        let audio_store = FakeStore::default();
        audio_store.map.lock().unwrap().insert("a1".into(), local(0.1, false));
        let audio_lane = SyncLane { enabled: false, source: &audio_src, store: &audio_store };

        // Reading lane errors → swallowed, local store untouched.
        let reading_src = FakeSource { remote: None, fail: true };
        let reading_store = FakeStore::default();
        reading_store.map.lock().unwrap().insert("e1".into(), local(0.3, false));
        let reading_lane = SyncLane { enabled: true, source: &reading_src, store: &reading_store };

        let books = vec![book("a1"), ebook("e1", "localfiles")];
        let report =
            reconcile_catalog(&books, Some(&audio_lane), Some(&reading_lane), 4).await;

        assert_eq!(report.pulled_down, 0);
        assert_eq!(report.failed, 1, "reading fetch error tallied, not thrown");
        // Disabled audio book was skipped entirely (not counted, store intact).
        assert!((audio_store.get_progress("a1").unwrap().fraction - 0.1).abs() < 1e-6);
        // Local reading store is the source of truth — untouched by the failed pull.
        assert!((reading_store.get_progress("e1").unwrap().fraction - 0.3).abs() < 1e-6);
    }

    // ---- manual conflict detection (is_genuine_conflict) -----------------

    #[test]
    fn conflict_only_on_divergent_untimestamped_positions() {
        // Divergent fractions, no timestamps → genuine conflict.
        assert!(is_genuine_conflict(&local(0.2, false), None, &remote(0.8, false, None)));
    }

    #[test]
    fn tiny_delta_is_not_a_conflict() {
        // Within PROGRESS_TIE_EPSILON → in sync, never a conflict.
        assert!(!is_genuine_conflict(&local(0.500, false), None, &remote(0.505, false, None)));
    }

    #[test]
    fn finished_side_is_never_a_conflict() {
        // Remote finished (sticky) → deterministic, not a conflict.
        assert!(!is_genuine_conflict(&local(0.2, false), None, &remote(0.9, true, None)));
        // Local finished → deterministic, not a conflict.
        assert!(!is_genuine_conflict(&local(1.0, true), None, &remote(0.2, false, None)));
    }

    #[test]
    fn confident_timestamp_ordering_is_not_a_conflict() {
        // Both timestamps present and far apart → newest-wins decides, no conflict.
        assert!(!is_genuine_conflict(&local(0.2, false), Some(1_000), &remote(0.8, false, Some(5_000))));
    }

    #[test]
    fn tied_timestamps_with_divergence_is_a_conflict() {
        // Timestamps within RECENCY_TIE_SECONDS → can't order → conflict.
        assert!(is_genuine_conflict(&local(0.2, false), Some(1_000), &remote(0.8, false, Some(1_001))));
    }

    // ---- manual-mode catalog (reconcile_catalog_with_policy) -------------

    #[tokio::test]
    async fn manual_mode_records_conflict_without_writing() {
        // Divergent, untimestamped audio positions → genuine conflict.
        let audio_src = FakeSource { remote: Some(remote(0.8, false, None)), fail: false };
        let audio_store = FakeStore::default();
        audio_store.map.lock().unwrap().insert("a1".into(), local(0.2, false));
        let audio_lane = SyncLane { enabled: true, source: &audio_src, store: &audio_store };

        let books = vec![book("a1")];
        let (report, conflicts) = reconcile_catalog_with_policy(
            &books,
            Some(&audio_lane),
            None,
            4,
            ConflictResolution::Manual,
        )
        .await;

        assert_eq!(report.conflicts, 1);
        assert_eq!(report.pulled_down, 0, "manual conflict is not auto-written");
        // Store is left untouched at the local value.
        assert!((audio_store.get_progress("a1").unwrap().fraction - 0.2).abs() < 1e-6);
        // The pending conflict carries both sides and both resolvable payloads.
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c.book_id, "a1");
        assert_eq!(c.lane, ConflictLane::Listening);
        assert!((c.local.fraction - 0.2).abs() < 1e-6);
        assert!((c.remote.fraction - 0.8).abs() < 1e-6);
        assert!((c.resolved(ConflictChoice::KeepLocal).fraction - 0.2).abs() < 1e-6);
        assert!((c.resolved(ConflictChoice::UseRemote).fraction - 0.8).abs() < 1e-6);
        assert!((c.resolved(ConflictChoice::KeepFurthest).fraction - 0.8).abs() < 1e-6);
    }

    #[tokio::test]
    async fn manual_mode_still_auto_resolves_clear_winners() {
        // Remote finished → clear winner, auto-applied even in manual mode.
        let audio_src = FakeSource { remote: Some(remote(1.0, true, None)), fail: false };
        let audio_store = FakeStore::default();
        audio_store.map.lock().unwrap().insert("a1".into(), local(0.2, false));
        let audio_lane = SyncLane { enabled: true, source: &audio_src, store: &audio_store };

        let books = vec![book("a1")];
        let (report, conflicts) = reconcile_catalog_with_policy(
            &books,
            Some(&audio_lane),
            None,
            4,
            ConflictResolution::Manual,
        )
        .await;

        assert_eq!(report.conflicts, 0, "finished-sticky is not a conflict");
        assert_eq!(report.pulled_down, 1);
        assert!(conflicts.is_empty());
        assert!(audio_store.get_progress("a1").unwrap().finished);
    }
}
