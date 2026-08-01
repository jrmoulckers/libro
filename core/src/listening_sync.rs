//! Listening-progress sync-back: mirror in-app audiobook progress **up** to the
//! user's Audiobookshelf server.
//!
//! This is the audio analog of [`crate::sync`] (the reading→Hardcover sync). When
//! the user makes listening progress locally (the `save_listening_progress`
//! path), Libro can — if the user opts in — reflect their position on the ABS
//! server so other devices/apps resume in the right place.
//!
//! ## Why a sibling module (not folded into [`crate::sync`])
//!
//! The two syncs share an *architecture* but not a *shape*:
//! * Reading sync must **resolve** a `Book` to a tracker's internal book id
//!   (ISBN/title search) and pushes **status transitions**
//!   (currently-reading → read).
//! * Listening sync targets the **same** item id it already has — the ABS
//!   `libraryItemId` *is* the audiobook `Book.id` (the id
//!   `get_audiobook_stream` opens a play session with) — so there is **no**
//!   resolve/search step, and it pushes a **position** (`currentTime` /
//!   `duration` / `isFinished`), throttled by a seconds delta rather than a
//!   status change.
//!
//! Keeping this a sibling module lets each engine stay focused on its own tracker
//! surface, throttle metric, and outcome type, instead of overloading the
//! reading engine's `ReadingStatus`/bucket model. The guarantees (opt-in,
//! failure isolation, throttle, interior-mutable state with the lock never held
//! across an `.await`) are identical.
//!
//! ## Guarantees
//!
//! * **Opt-in.** Gated on an `enabled` flag (wired to
//!   `AudiobookshelfConfig::sync_listening_progress`, default `false`) and on a
//!   tracker actually being configured. Off ⇒ zero calls.
//! * **Failure isolation.** The local [`crate::config::ListeningStore`] save is
//!   the source of truth and happens first, in the caller. Every error here is
//!   captured into a [`ListeningSyncOutcome`] and *never* propagated.
//! * **Throttled.** Per-item last-synced position means we only call ABS on a
//!   meaningful move (≥ [`MIN_POSITION_DELTA_SECONDS`]) or on finish — never on
//!   every `timeupdate`.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::models::Progress;
use crate::providers::ProviderResult;

/// Minimum change in position (seconds) before we push another update. Ordinary
/// `timeupdate` ticks move only a second or two, so this keeps the API quiet
/// while still capturing meaningful seeks/listening.
pub const MIN_POSITION_DELTA_SECONDS: f64 = 15.0;

/// The minimal write surface the sync engine needs from an audiobook server.
///
/// Implemented by `AudiobookshelfProvider` and by fakes in tests. Best-effort
/// from the engine's perspective — any `Err` is caught and folded into a
/// [`ListeningSyncOutcome`].
#[async_trait]
pub trait ListeningTracker: Send + Sync {
    /// Push the listening position for one server item (the ABS `libraryItemId`).
    async fn update_media_progress(
        &self,
        item_id: &str,
        position_seconds: f64,
        duration_seconds: Option<f64>,
        is_finished: bool,
    ) -> ProviderResult<()>;
}

/// What a single listening-sync attempt did — returned (never thrown) so the
/// caller can log it and move on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListeningSyncOutcome {
    /// Sync is turned off for this server (opt-in flag false).
    Disabled,
    /// No tracker is configured (e.g. ABS not set up, or the book isn't from ABS).
    NotConfigured,
    /// Nothing to do — unstarted, or too small a move since last sync (throttled).
    NoChange,
    /// A position update was pushed.
    Updated,
    /// The item was reported finished (pushed once).
    Finished,
    /// A tracker/network error occurred and was swallowed; message is for logs.
    Failed(String),
}

/// Per-item state we remember between saves so we can throttle API calls.
#[derive(Debug, Clone, Copy)]
struct SyncedAudio {
    position: f64,
    finished: bool,
}

#[derive(Default)]
struct Inner {
    /// ABS `libraryItemId` → last-synced position/finished, for throttle detection.
    last: HashMap<String, SyncedAudio>,
}

/// Cross-call throttle state for the listening-sync engine.
///
/// Held once (e.g. in Tauri managed state) and shared across every
/// [`sync_listening_progress`] call so the per-item last-synced position persists
/// for the life of the app process. Interior mutability so callers need only
/// `&self`; the lock is only ever held briefly, never across an `.await`.
#[derive(Default)]
pub struct ListeningSyncState {
    inner: Mutex<Inner>,
}

impl ListeningSyncState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an *externally-applied* position (e.g. one just pulled **down** by
    /// the inbound reconciliation, [`crate::progress_sync`]) as the last-synced
    /// state for `item_id`, without any API call. This stops the next in-app save
    /// at that same position from triggering a spurious outward push — the two
    /// directions share this throttle state, so a pulled-down value is not
    /// immediately echoed back up.
    pub fn note_synced_position(&self, item_id: &str, position_seconds: f64, finished: bool) {
        self.inner.lock().unwrap().last.insert(
            item_id.to_string(),
            SyncedAudio {
                position: position_seconds,
                finished,
            },
        );
    }
}

/// Whether a local audio [`Progress`] counts as finished.
fn is_finished(p: &Progress) -> bool {
    p.finished || p.fraction >= 0.99
}

/// Best-guess total duration (seconds) from a local audio [`Progress`]:
/// `position / fraction`. `None` when it can't be derived (no position, or
/// unstarted). Near the end this converges on the real duration.
fn derive_duration(p: &Progress) -> Option<f64> {
    let pos = p.position_seconds?;
    if p.fraction > 0.0 {
        Some(pos / p.fraction as f64)
    } else {
        None
    }
}

/// Best-effort push of local listening progress to an audiobook server.
///
/// Never returns an error: all failures are captured in the returned
/// [`ListeningSyncOutcome`]. The caller must have already persisted the local
/// progress (that is the source of truth); this only mirrors it outward.
///
/// `item_id` is the ABS `libraryItemId` (the audiobook `Book.id`). See the module
/// docs for the opt-in, failure-isolation, and throttle guarantees.
pub async fn sync_listening_progress<T: ListeningTracker + ?Sized>(
    enabled: bool,
    tracker: Option<&T>,
    state: &ListeningSyncState,
    item_id: &str,
    progress: &Progress,
) -> ListeningSyncOutcome {
    if !enabled {
        return ListeningSyncOutcome::Disabled;
    }
    let Some(tracker) = tracker else {
        return ListeningSyncOutcome::NotConfigured;
    };

    let position = progress.position_seconds.unwrap_or(0.0);
    let finished = is_finished(progress);

    // Nothing to reflect for an unstarted item (position 0 and not finished).
    if position <= 0.0 && !finished {
        return ListeningSyncOutcome::NoChange;
    }

    // Throttle: push on the first meaningful save, a finish transition, or a
    // position move past the threshold. The lock is dropped before the await.
    let last = { state.inner.lock().unwrap().last.get(item_id).copied() };
    let should_push = match last {
        None => true,
        Some(prev) => {
            if finished {
                !prev.finished // finish pushes exactly once
            } else {
                (position - prev.position).abs() >= MIN_POSITION_DELTA_SECONDS
            }
        }
    };
    if !should_push {
        return ListeningSyncOutcome::NoChange;
    }

    let duration = derive_duration(progress);
    match tracker
        .update_media_progress(item_id, position, duration, finished)
        .await
    {
        Ok(()) => {
            state
                .inner
                .lock()
                .unwrap()
                .last
                .insert(item_id.to_string(), SyncedAudio { position, finished });
            if finished {
                ListeningSyncOutcome::Finished
            } else {
                ListeningSyncOutcome::Updated
            }
        }
        // Don't record state on failure, so a later save retries.
        Err(e) => ListeningSyncOutcome::Failed(format!("update_media_progress failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderError;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn audio_progress(position: f64, fraction: f32, finished: bool) -> Progress {
        Progress {
            fraction,
            position_seconds: Some(position),
            locator: None,
            finished,
        }
    }

    /// A network-free listening tracker that counts calls, records the last body,
    /// and can be told to fail.
    #[derive(Default)]
    struct FakeListeningTracker {
        calls: AtomicUsize,
        last: Mutex<Option<(String, f64, Option<f64>, bool)>>,
        fail: bool,
    }

    impl FakeListeningTracker {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ListeningTracker for FakeListeningTracker {
        async fn update_media_progress(
            &self,
            item_id: &str,
            position_seconds: f64,
            duration_seconds: Option<f64>,
            is_finished: bool,
        ) -> ProviderResult<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(ProviderError::Network("boom".into()));
            }
            *self.last.lock().unwrap() =
                Some((item_id.to_string(), position_seconds, duration_seconds, is_finished));
            Ok(())
        }
    }

    #[tokio::test]
    async fn zero_position_is_a_noop_then_first_real_position_updates() {
        let t = FakeListeningTracker::default();
        let state = ListeningSyncState::new();

        // Unstarted → nothing to push.
        let out =
            sync_listening_progress(true, Some(&t), &state, "li_1", &audio_progress(0.0, 0.0, false))
                .await;
        assert_eq!(out, ListeningSyncOutcome::NoChange);
        assert_eq!(t.calls(), 0);

        // First real position → one update.
        let out =
            sync_listening_progress(true, Some(&t), &state, "li_1", &audio_progress(30.0, 0.02, false))
                .await;
        assert_eq!(out, ListeningSyncOutcome::Updated);
        assert_eq!(t.calls(), 1);
        let last = t.last.lock().unwrap().clone().unwrap();
        assert_eq!(last.0, "li_1");
        assert_eq!(last.1, 30.0);
        assert_eq!(last.3, false);
    }

    #[tokio::test]
    async fn a_small_delta_below_threshold_does_not_call_the_api() {
        let t = FakeListeningTracker::default();
        let state = ListeningSyncState::new();

        sync_listening_progress(true, Some(&t), &state, "li_1", &audio_progress(30.0, 0.02, false))
            .await; // first update
        // +10s < 15s threshold → throttled.
        let out =
            sync_listening_progress(true, Some(&t), &state, "li_1", &audio_progress(40.0, 0.03, false))
                .await;
        assert_eq!(out, ListeningSyncOutcome::NoChange);
        assert_eq!(t.calls(), 1);
    }

    #[tokio::test]
    async fn crossing_the_threshold_pushes_one_update() {
        let t = FakeListeningTracker::default();
        let state = ListeningSyncState::new();

        sync_listening_progress(true, Some(&t), &state, "li_1", &audio_progress(30.0, 0.02, false))
            .await; // first update
        // +20s ≥ 15s threshold → one more update.
        let out =
            sync_listening_progress(true, Some(&t), &state, "li_1", &audio_progress(50.0, 0.04, false))
                .await;
        assert_eq!(out, ListeningSyncOutcome::Updated);
        assert_eq!(t.calls(), 2);
    }

    #[tokio::test]
    async fn finish_sends_is_finished_exactly_once() {
        let t = FakeListeningTracker::default();
        let state = ListeningSyncState::new();

        let out = sync_listening_progress(
            true,
            Some(&t),
            &state,
            "li_1",
            &audio_progress(3600.0, 1.0, true),
        )
        .await;
        assert_eq!(out, ListeningSyncOutcome::Finished);
        let last = t.last.lock().unwrap().clone().unwrap();
        assert_eq!(last.3, true, "isFinished should be true");

        // A repeat finished save must not call the API again.
        let out2 = sync_listening_progress(
            true,
            Some(&t),
            &state,
            "li_1",
            &audio_progress(3600.0, 1.0, true),
        )
        .await;
        assert_eq!(out2, ListeningSyncOutcome::NoChange);
        assert_eq!(t.calls(), 1);
    }

    #[tokio::test]
    async fn disabled_flag_makes_zero_calls() {
        let t = FakeListeningTracker::default();
        let state = ListeningSyncState::new();
        let out =
            sync_listening_progress(false, Some(&t), &state, "li_1", &audio_progress(30.0, 0.5, false))
                .await;
        assert_eq!(out, ListeningSyncOutcome::Disabled);
        assert_eq!(t.calls(), 0);
    }

    #[tokio::test]
    async fn not_configured_makes_zero_calls() {
        let state = ListeningSyncState::new();
        let out = sync_listening_progress::<FakeListeningTracker>(
            true,
            None,
            &state,
            "li_1",
            &audio_progress(30.0, 0.5, false),
        )
        .await;
        assert_eq!(out, ListeningSyncOutcome::NotConfigured);
    }

    #[tokio::test]
    async fn erroring_tracker_is_swallowed_and_local_store_stays_intact() {
        use crate::config::ListeningStore;

        let t = FakeListeningTracker {
            fail: true,
            ..Default::default()
        };
        let state = ListeningSyncState::new();
        let p = audio_progress(120.0, 0.1, false);

        // Mirror the command's ordering: the local store is written first and is
        // the source of truth.
        let tmp = tempfile::tempdir().unwrap();
        let store = ListeningStore::new(tmp.path().join("listening.json"));
        store.save("li_1", p.clone()).unwrap();

        let out = sync_listening_progress(true, Some(&t), &state, "li_1", &p).await;
        assert!(matches!(out, ListeningSyncOutcome::Failed(_)));

        // The failed sync must not have disturbed the persisted local progress,
        // and (state not recorded on failure) a later save may retry.
        let got = store.get("li_1").unwrap().expect("local progress intact");
        assert_eq!(got, p);
    }

    #[tokio::test]
    async fn derive_duration_is_position_over_fraction() {
        assert_eq!(derive_duration(&audio_progress(1800.0, 0.5, false)), Some(3600.0));
        assert_eq!(derive_duration(&audio_progress(0.0, 0.0, false)), None);
    }
}
