//! Reading-progress sync: mirror in-app reading progress to a tracking service.
//!
//! This ties the **reading** phase to the **tracking** phase. When the user makes
//! reading progress locally (the `save_reading_progress` path), Libro can — if the
//! user opts in — reflect that on their tracking account: mark a book
//! *currently-reading* when they start it and *read* when they finish.
//!
//! ## Design
//!
//! The write surface a tracker must expose is abstracted behind the
//! [`ProgressTracker`] trait, which [`HardcoverProvider`] implements. The sync
//! logic ([`sync_reading_progress`]) depends only on the trait, so it is fully
//! unit-testable against a fake tracker with **no network**.
//!
//! ## Guarantees
//!
//! * **Opt-in.** Gated on an `enabled` flag (wired to
//!   `HardcoverConfig::sync_reading_progress`, default `false`) and on a tracker
//!   actually being configured. Off ⇒ zero calls.
//! * **Failure isolation.** The local save is the source of truth and happens
//!   first, in the caller. Every error here (resolve, network, API) is captured
//!   into a [`SyncOutcome`] and *never* propagated, so it can't break saving or
//!   the reader.
//! * **Throttled.** Per-book last-synced state (status + a coarse progress bucket)
//!   plus a resolve cache mean we only call the API on a real transition (start,
//!   finish) or a meaningful progress delta — never on every page turn.
//!
//! [`HardcoverProvider`]: crate::providers::hardcover::HardcoverProvider

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::models::{Book, Progress};
use crate::providers::hardcover::ReadingStatus;
use crate::providers::ProviderResult;

/// The minimal write surface the sync engine needs from a reading tracker.
///
/// Implemented by real connectors (e.g. `HardcoverProvider`) and by fakes in
/// tests. All methods are best-effort from the engine's perspective — any `Err`
/// is caught and folded into a [`SyncOutcome`].
#[async_trait]
pub trait ProgressTracker: Send + Sync {
    /// Resolve a normalized [`Book`] to the tracker's internal book id, or `None`
    /// when nothing matches.
    async fn resolve_book_id(&self, book: &Book) -> ProviderResult<Option<i64>>;

    /// Set the reading status for a tracker book id. Returns the tracker's
    /// `user_book` id when the API echoes one back (used for later progress
    /// updates), else `None`.
    async fn set_status(
        &self,
        book_id: i64,
        status: ReadingStatus,
    ) -> ProviderResult<Option<i64>>;

    /// Record a reading-progress delta (fraction `0.0..=1.0`) against a
    /// `user_book` entry. Best-effort and optional.
    async fn update_progress(&self, user_book_id: i64, fraction: f32) -> ProviderResult<()>;
}

/// What a single sync attempt did — returned (never thrown) so the caller can log
/// it and move on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Sync is turned off for this tracker (opt-in flag false).
    Disabled,
    /// No tracker is configured (e.g. no Hardcover API key).
    NotConfigured,
    /// The book couldn't be matched to a tracker book id.
    Unresolved,
    /// Nothing to do — same status and progress bucket as last time (throttled).
    NoChange,
    /// A status transition was pushed (e.g. currently-reading or read).
    StatusSet(ReadingStatus),
    /// Only a progress-record update was pushed (status unchanged).
    ProgressOnly,
    /// A tracker/network error occurred and was swallowed; message is for logs.
    Failed(String),
}

/// Per-book state we remember between saves so we can throttle API calls.
#[derive(Debug, Clone, Copy)]
struct SyncedState {
    status: ReadingStatus,
    bucket: i32,
    user_book_id: Option<i64>,
}

#[derive(Default)]
struct Inner {
    /// local `Book.id` → resolved tracker book id (`None` = resolved-but-unmatched,
    /// cached so we don't re-search a book we already know isn't on the service).
    resolved: HashMap<String, Option<i64>>,
    /// local `Book.id` → last-synced state, for transition/throttle detection.
    last: HashMap<String, SyncedState>,
}

/// Cross-call caches for the sync engine (resolve cache + last-synced state).
///
/// Held once (e.g. in Tauri managed state) and shared across every
/// [`sync_reading_progress`] call so throttling and id resolution persist for the
/// life of the app process. Uses interior mutability so callers only need `&self`;
/// the lock is only ever held briefly, never across an `.await`.
#[derive(Default)]
pub struct ReadingSyncState {
    inner: Mutex<Inner>,
}

impl ReadingSyncState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Map local reading [`Progress`] to the tracker status it should imply, or `None`
/// when there's nothing meaningful to reflect yet (unstarted).
fn desired_status(p: &Progress) -> Option<ReadingStatus> {
    if p.finished || p.fraction >= 0.99 {
        Some(ReadingStatus::Read)
    } else if p.fraction > 0.0 {
        Some(ReadingStatus::CurrentlyReading)
    } else {
        None
    }
}

/// Coarse 5%-wide progress bucket, so we only push a progress-record update when
/// the reader has moved a meaningful amount, not on every page turn.
fn progress_bucket(fraction: f32) -> i32 {
    (fraction.clamp(0.0, 1.0) * 20.0).floor() as i32
}

/// Best-effort push of local reading progress to a tracker.
///
/// This never returns an error: all failures are captured in the returned
/// [`SyncOutcome`]. The caller must have already persisted the local progress
/// (that is the source of truth); this only mirrors it outward.
///
/// See the module docs for the opt-in, failure-isolation, and throttle guarantees.
pub async fn sync_reading_progress<T: ProgressTracker + ?Sized>(
    enabled: bool,
    tracker: Option<&T>,
    state: &ReadingSyncState,
    book: &Book,
    progress: &Progress,
) -> SyncOutcome {
    if !enabled {
        return SyncOutcome::Disabled;
    }
    let Some(tracker) = tracker else {
        return SyncOutcome::NotConfigured;
    };

    // Nothing to reflect for an unstarted book.
    let Some(desired) = desired_status(progress) else {
        return SyncOutcome::NoChange;
    };

    let key = book.id.clone();

    // Resolve the tracker book id (cache-first; the lock is dropped before the
    // network call).
    let cached = { state.inner.lock().unwrap().resolved.get(&key).copied() };
    let resolved = match cached {
        Some(v) => v,
        None => match tracker.resolve_book_id(book).await {
            Ok(v) => {
                state.inner.lock().unwrap().resolved.insert(key.clone(), v);
                v
            }
            // Don't cache a transient error — allow a later retry.
            Err(e) => return SyncOutcome::Failed(format!("resolve failed: {e}")),
        },
    };
    let Some(tracker_book_id) = resolved else {
        return SyncOutcome::Unresolved;
    };

    let last = { state.inner.lock().unwrap().last.get(&key).copied() };
    let bucket = progress_bucket(progress.fraction);

    let mut user_book_id = last.and_then(|l| l.user_book_id);
    let status_changed = last.map(|l| l.status) != Some(desired);

    if status_changed {
        match tracker.set_status(tracker_book_id, desired).await {
            Ok(ub) => {
                if ub.is_some() {
                    user_book_id = ub;
                }
            }
            Err(e) => return SyncOutcome::Failed(format!("set_status failed: {e}")),
        }
    }

    // Only push a progress-record update on a real bucket change, and only when we
    // didn't just set status (a status write already reflects the position). This
    // is what keeps ordinary page turns from hitting the API.
    let bucket_changed = last.map(|l| l.bucket) != Some(bucket);
    let mut progress_updated = false;
    if bucket_changed && !status_changed {
        if let Some(ubid) = user_book_id {
            match tracker.update_progress(ubid, progress.fraction).await {
                Ok(()) => progress_updated = true,
                Err(e) => return SyncOutcome::Failed(format!("update_progress failed: {e}")),
            }
        }
    }

    {
        let mut inner = state.inner.lock().unwrap();
        inner.last.insert(
            key,
            SyncedState {
                status: desired,
                bucket,
                user_book_id,
            },
        );
    }

    if status_changed {
        SyncOutcome::StatusSet(desired)
    } else if progress_updated {
        SyncOutcome::ProgressOnly
    } else {
        SyncOutcome::NoChange
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderError;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn book_with_isbn(id: &str, isbn: &str) -> Book {
        let mut b = Book::new(
            id.to_string(),
            "Effective Java".to_string(),
            crate::models::MediaType::Ebook,
            "localfiles".to_string(),
        );
        b.authors = vec!["Joshua Bloch".to_string()];
        b.identifiers.insert("isbn".to_string(), isbn.to_string());
        b
    }

    fn progress(fraction: f32, finished: bool) -> Progress {
        Progress {
            fraction,
            position_seconds: None,
            locator: Some("epubcfi(/6/2!/4)".to_string()),
            finished,
        }
    }

    /// A network-free tracker double that counts calls and can be told to fail.
    #[derive(Default)]
    struct FakeTracker {
        resolve_calls: AtomicUsize,
        set_status_calls: AtomicUsize,
        update_calls: AtomicUsize,
        last_status: Mutex<Option<ReadingStatus>>,
        /// Resolve returns this id; `None` models "not on the service".
        resolve_to: Option<i64>,
        /// user_book id echoed back from set_status.
        user_book_id: Option<i64>,
        fail_resolve: bool,
        fail_set_status: bool,
    }

    impl FakeTracker {
        fn matching() -> Self {
            Self {
                resolve_to: Some(555),
                user_book_id: Some(999),
                ..Default::default()
            }
        }
        fn resolves(&self) -> usize {
            self.resolve_calls.load(Ordering::SeqCst)
        }
        fn statuses(&self) -> usize {
            self.set_status_calls.load(Ordering::SeqCst)
        }
        fn updates(&self) -> usize {
            self.update_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ProgressTracker for FakeTracker {
        async fn resolve_book_id(&self, _book: &Book) -> ProviderResult<Option<i64>> {
            self.resolve_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_resolve {
                return Err(ProviderError::Network("boom".into()));
            }
            Ok(self.resolve_to)
        }

        async fn set_status(
            &self,
            _book_id: i64,
            status: ReadingStatus,
        ) -> ProviderResult<Option<i64>> {
            self.set_status_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_set_status {
                return Err(ProviderError::Api("nope".into()));
            }
            *self.last_status.lock().unwrap() = Some(status);
            Ok(self.user_book_id)
        }

        async fn update_progress(&self, _user_book_id: i64, _fraction: f32) -> ProviderResult<()> {
            self.update_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn start_sets_currently_reading_exactly_once() {
        let t = FakeTracker::matching();
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");

        let out = sync_reading_progress(true, Some(&t), &state, &book, &progress(0.3, false)).await;
        assert_eq!(out, SyncOutcome::StatusSet(ReadingStatus::CurrentlyReading));

        // A second save at the same state must not call the API again.
        let out2 = sync_reading_progress(true, Some(&t), &state, &book, &progress(0.3, false)).await;
        assert_eq!(out2, SyncOutcome::NoChange);

        assert_eq!(t.resolves(), 1, "resolve should be cached after the first call");
        assert_eq!(t.statuses(), 1, "status should be set exactly once");
        assert_eq!(*t.last_status.lock().unwrap(), Some(ReadingStatus::CurrentlyReading));
    }

    #[tokio::test]
    async fn finish_sets_read() {
        let t = FakeTracker::matching();
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");

        let out = sync_reading_progress(true, Some(&t), &state, &book, &progress(1.0, true)).await;
        assert_eq!(out, SyncOutcome::StatusSet(ReadingStatus::Read));
        assert_eq!(*t.last_status.lock().unwrap(), Some(ReadingStatus::Read));
    }

    #[tokio::test]
    async fn currently_reading_then_finish_is_two_transitions() {
        let t = FakeTracker::matching();
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");

        sync_reading_progress(true, Some(&t), &state, &book, &progress(0.2, false)).await;
        let out = sync_reading_progress(true, Some(&t), &state, &book, &progress(1.0, true)).await;
        assert_eq!(out, SyncOutcome::StatusSet(ReadingStatus::Read));
        assert_eq!(t.statuses(), 2);
        assert_eq!(t.resolves(), 1, "resolve stays cached across transitions");
    }

    #[tokio::test]
    async fn repeated_page_turns_within_a_bucket_do_not_call_the_api() {
        let t = FakeTracker::matching();
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");

        // Start (one status call), then nudge within the same 5% bucket repeatedly.
        sync_reading_progress(true, Some(&t), &state, &book, &progress(0.30, false)).await;
        for f in [0.31_f32, 0.32, 0.33, 0.34] {
            let out =
                sync_reading_progress(true, Some(&t), &state, &book, &progress(f, false)).await;
            assert_eq!(out, SyncOutcome::NoChange);
        }
        assert_eq!(t.statuses(), 1);
        assert_eq!(t.updates(), 0, "no progress-record spam within a bucket");
    }

    #[tokio::test]
    async fn crossing_a_bucket_pushes_a_progress_update_once() {
        let t = FakeTracker::matching();
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");

        sync_reading_progress(true, Some(&t), &state, &book, &progress(0.30, false)).await; // start
        let out = sync_reading_progress(true, Some(&t), &state, &book, &progress(0.55, false)).await;
        assert_eq!(out, SyncOutcome::ProgressOnly);
        assert_eq!(t.updates(), 1);
        assert_eq!(t.statuses(), 1, "still currently-reading, no new status");
    }

    #[tokio::test]
    async fn disabled_flag_does_nothing() {
        let t = FakeTracker::matching();
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");

        let out = sync_reading_progress(false, Some(&t), &state, &book, &progress(0.5, false)).await;
        assert_eq!(out, SyncOutcome::Disabled);
        assert_eq!(t.resolves(), 0);
        assert_eq!(t.statuses(), 0);
    }

    #[tokio::test]
    async fn not_configured_does_nothing() {
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");
        let out = sync_reading_progress::<FakeTracker>(
            true,
            None,
            &state,
            &book,
            &progress(0.5, false),
        )
        .await;
        assert_eq!(out, SyncOutcome::NotConfigured);
    }

    #[tokio::test]
    async fn unresolved_book_is_cached_and_not_retried() {
        let t = FakeTracker::default(); // resolve_to = None → not on the service
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "0000000000000");

        let out = sync_reading_progress(true, Some(&t), &state, &book, &progress(0.5, false)).await;
        assert_eq!(out, SyncOutcome::Unresolved);
        // Second save: resolve is cached as "unmatched", so no second search.
        let out2 = sync_reading_progress(true, Some(&t), &state, &book, &progress(0.6, false)).await;
        assert_eq!(out2, SyncOutcome::Unresolved);
        assert_eq!(t.resolves(), 1);
        assert_eq!(t.statuses(), 0);
    }

    #[tokio::test]
    async fn tracker_error_is_swallowed_and_local_save_survives() {
        use crate::config::ReadingStore;

        let t = FakeTracker {
            resolve_to: Some(1),
            fail_set_status: true,
            ..Default::default()
        };
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");
        let p = progress(0.4, false);

        // Mirror the command's ordering: local save is the source of truth first.
        let tmp = tempfile::tempdir().unwrap();
        let store = ReadingStore::new(tmp.path().join("reading.json"));
        store.save(&book.id, p.clone()).unwrap();

        let out = sync_reading_progress(true, Some(&t), &state, &book, &p).await;
        assert!(matches!(out, SyncOutcome::Failed(_)));

        // The failed sync must not have disturbed the persisted local progress.
        let got = store.get(&book.id).unwrap().expect("local progress intact");
        assert_eq!(got, p);
    }

    #[tokio::test]
    async fn resolve_error_does_not_poison_the_cache() {
        let mut t = FakeTracker::matching();
        t.fail_resolve = true;
        let state = ReadingSyncState::new();
        let book = book_with_isbn("b1", "9780134685991");

        let out = sync_reading_progress(true, Some(&t), &state, &book, &progress(0.5, false)).await;
        assert!(matches!(out, SyncOutcome::Failed(_)));
        // A later working resolve is still attempted (nothing cached from the error).
        let ok = FakeTracker::matching();
        let out2 = sync_reading_progress(true, Some(&ok), &state, &book, &progress(0.5, false)).await;
        assert_eq!(out2, SyncOutcome::StatusSet(ReadingStatus::CurrentlyReading));
    }
}
