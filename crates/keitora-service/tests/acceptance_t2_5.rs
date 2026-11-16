// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.5: the service engine, which wires
//! `keitora-core`'s pomodoro engine to the store, applies client requests,
//! emits events and persists a snapshot on every transition.
//!
//! This file covers requests, strict-mode rules, events, session ends,
//! blocklists, settings, paging, logs and the wiring into the pipe server and
//! the running service. What the store holds after each transition, issue
//! #48 (a full disk must not erase a strict session) and issue #49 (bind the
//! snapshot to its blocklist) are in `acceptance_t2_5_persistence.rs`.
//!
//! # The API these tests expect (module `keitora_service::engine`)
//!
//! ```text
//! pub trait EventSink: Send + Sync {
//!     /// Delivers one event to the subscribers. Never blocks on a client.
//!     fn publish(&self, event: keitora_ipc::Event);
//! }
//! pub struct ServiceEngine { .. }           // Send + Sync + 'static
//! impl ServiceEngine {
//!     pub fn new(
//!         store: keitora_service::store::Store,
//!         clock: Arc<dyn keitora_core::clock::Clock + Send + Sync>,
//!         events: Arc<dyn EventSink>,
//!     ) -> Result<ServiceEngine, E>;        // E: Debug, the developer's type
//!     /// Observes the clock: publishes the transitions that happened since
//!     /// the last call, records the history of a session that ended, and
//!     /// saves a checkpoint when one is due or a transition happened, or
//!     /// retries one that failed. Synchronous: when it returns, that is done.
//!     pub fn tick(&self);
//! }
//! impl keitora_service::pipe::RequestHandler for ServiceEngine { .. }
//! ```
//!
//! - The engine reads time only from the clock it is given, and has no
//!   thread of its own: between calls to `handle` and `tick` it does nothing,
//!   so the tests decide exactly when time is observed. The running service
//!   creates one `SystemClock`, gives it to the engine, and calls `tick`
//!   about once a second.
//! - Mutating requests (`StartSession`, `StopSession`, `PauseSession`,
//!   `ResumeSession`, `SkipBreak`, `UpsertBlocklist`, `DeleteBlocklist`,
//!   `UpdateSettings`) answer `Response::Ok` on success, as the desktop data
//!   layer expects (ADR-0016: they resolve to `void`). Reads answer
//!   `State`, `Blocklists`, `Blocklist`, `Settings`, `History` and
//!   `BlockEvents`.
//!
//! # Decisions taken for this task (standing authorization)
//!
//! - **Error codes.** A request that needs a session when there is none, a
//!   second pause, a resume of a running session, a skip during a focus and
//!   a start over a running unlocked session are `invalid_state`. Any
//!   request refused because a strict session is active, including a start
//!   over it, is `strict_session_locked { reason: until_timer_ends }`. An
//!   unknown blocklist id is `not_found`, also for `DeleteBlocklist`. Ids are
//!   compared exactly, so case matters.
//! - **Blocklist edits during a strict session.** `can_edit_blocklist` takes
//!   no id, and the roadmap asks it to cover any blocklist: while a strict
//!   session is active every `UpsertBlocklist` and `DeleteBlocklist` is
//!   refused, the session's own list, any other list and a new one alike.
//!   Unlocked and `RandomText` sessions allow every edit.
//! - **Settings during a strict session.** `Settings` holds only
//!   `default_preset`, which applies to sessions started later. So
//!   `UpdateSettings` is allowed during a strict session and never changes
//!   the running one.
//! - **`RandomText` (F1 review, L-f).** Stopping needs the exact text; a
//!   missing or wrong text is `validation` for the field `text`, and the
//!   session goes on. Pausing and skipping a break don't need the text: the
//!   roadmap (T-1.6) applies the strict rules to `UntilTimerEnds` alone, and
//!   `PauseSession` and `SkipBreak` carry no text.
//! - **Events.** Every start, pause, resume, skip, stop and phase change
//!   publishes `StateChanged` with the state `GetState` reports right after.
//!   A stop publishes `SessionEnded { stopped }` and an idle `StateChanged`;
//!   a session that runs out publishes `SessionEnded { completed }` and an
//!   idle `StateChanged`. Creating, changing or deleting a blocklist
//!   publishes `BlocklistChanged { id }`. A refused request and a tick with
//!   no transition publish nothing.
//! - **A session that ended unobserved.** If a session ran out and a request
//!   arrives before the next tick, its end is still published and recorded,
//!   exactly once, before the request is applied.
//! - **History.** An entry's start time is the injected clock's wall reading
//!   when the session started; its focus time is the focus actually run.
//! - **Paging.** Blocklists are listed in the store's order. `next_offset`
//!   is the offset after the page, or `None` when no item is left after it.
//!   Any offset, up to `u32::MAX`, is answered without overflowing.
//!
//! # Criteria covered
//!
//! - **EN1** — the engine takes its time from the injected clock only, and
//!   no request carries a time:
//!   `get_state_reports_idle_before_any_session`,
//!   `remaining_time_comes_from_the_injected_monotonic_clock`,
//!   `history_takes_the_start_time_from_the_injected_wall_clock`.
//! - **EN2** — session requests are applied through `keitora-core`'s engine:
//!   `start_session_runs_the_first_focus_of_the_requested_preset_and_lock`,
//!   `phases_follow_the_clock`,
//!   `start_session_with_an_unknown_blocklist_is_not_found_and_starts_nothing`,
//!   `blocklist_ids_are_compared_exactly`,
//!   `start_session_over_a_running_unlocked_session_is_invalid_state`,
//!   `pause_and_resume_freeze_and_continue_the_timer`,
//!   `session_requests_without_a_session_are_invalid_state`,
//!   `a_second_pause_or_a_resume_while_running_is_invalid_state`,
//!   `skip_break_during_a_focus_is_invalid_state`,
//!   `skip_break_during_a_short_break_starts_the_next_focus`,
//!   `stop_ends_an_unlocked_session`.
//! - **EN3** (threat model, #18) — no request can shorten, pause, cancel or
//!   restart an active strict session, or edit a blocklist while it runs;
//!   settings can't touch it; everything is allowed again once it ends:
//!   `a_strict_session_refuses_stop_pause_skip_and_another_start`,
//!   `a_strict_session_refuses_editing_or_deleting_its_blocklist`,
//!   `a_strict_session_refuses_editing_deleting_or_creating_any_other_blocklist`,
//!   `updating_settings_during_a_strict_session_doesnt_touch_it`,
//!   `a_strict_session_unlocks_everything_once_its_timer_ends`,
//!   `an_unlocked_session_allows_editing_its_blocklist`.
//! - **EN4** — a `RandomText` session stops only with its text, which is
//!   never logged; pause and skip need no text:
//!   `a_random_text_session_refuses_a_stop_without_its_text`,
//!   `the_typed_stop_text_is_never_logged`,
//!   `a_random_text_session_can_be_paused_and_its_break_skipped_without_text`.
//! - **EN5** — every transition publishes the matching event, and nothing
//!   else does:
//!   `starting_a_session_publishes_its_new_state`,
//!   `pause_resume_and_skip_publish_the_new_state`,
//!   `stopping_publishes_session_ended_and_the_idle_state`,
//!   `a_tick_publishes_each_phase_change`,
//!   `a_tick_without_a_transition_publishes_nothing`,
//!   `refused_requests_publish_nothing`,
//!   `blocklist_edits_publish_blocklist_changed`.
//! - **EN6** — the end of a session is published and recorded in the
//!   history:
//!   `a_session_that_runs_out_publishes_session_ended_and_is_recorded_as_completed`,
//!   `a_stopped_session_is_recorded_as_stopped_with_its_focus_time`,
//!   `a_session_that_ended_between_ticks_is_reported_once_before_the_next_start`.
//! - **EN7** — blocklists, settings, history and block events are served
//!   from the store, paged, with any offset:
//!   `blocklists_can_be_created_read_and_deleted`,
//!   `deleting_an_unknown_blocklist_is_not_found`,
//!   `list_blocklists_pages_in_the_stored_order`,
//!   `paged_requests_with_a_huge_offset_return_an_empty_last_page`,
//!   `get_block_events_pages_the_stored_events`,
//!   `settings_start_at_the_default_and_can_be_updated`.
//! - **EN8** — logs never hold user data: `blocklists_are_never_logged` (and
//!   `the_typed_stop_text_is_never_logged`, EN4).
//! - **EN9** — the engine is what the service runs: it serves the pipe as
//!   its request handler, and the console service creates it with its store
//!   on first start:
//!   `the_engine_answers_requests_through_the_pipe_server`,
//!   `the_console_service_serves_the_engine_on_its_pipe_and_saves_an_idle_snapshot`
//!   (compiled only with debug assertions, like T-2.1's `--console` tests).
//! - Harness check, no criterion: `log_capture_sees_this_threads_events`
//!   shows that the log tests can see what is logged.
//!
//! # Not covered: the protocol can't carry it
//!
//! No response or event tells a client the text of a `RandomText` session,
//! so no client, and no test, can stop one with the right text: only the
//! refusals are tested. Carrying the text needs a change to `keitora-ipc`,
//! which this task doesn't make (reported to the maintainer).
//!
//! # Verified by review, not here
//!
//! - The service creates exactly one `SystemClock` and passes it to the
//!   engine; it never builds the core engine with `Engine::default()` or
//!   `Engine::new()` in place of a restored or running one; `FakeClock`
//!   appears in no production code.
//! - `LockKind::is_strict` in `keitora-core` is an exhaustive `match`.
//! - The `RandomText` text is generated by the service from a
//!   cryptographically secure source and compared in constant time.
//! - `StoreError` is logged with its kind, never with SQLite's message text
//!   from a planted file; no log line holds a blocklist, a setting, a block
//!   event's target or typed text.
//! - The running service calls `tick` about once a second, and logging
//!   never blocks it for long.

#![cfg(test)]

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keitora_core::engine::{LockKind, PomodoroPreset};
use keitora_core::model::{Blocklist, SitePattern, Target};
use keitora_ipc::{
    BlockEvent, BlocklistSummary, Envelope, Event, FrameReader, HistoryEntry, HistoryRange,
    LockReason, Message, MessageId, Outcome, PageRequest, Phase, ProtocolError, Request, Response,
    SessionState, Settings, TargetKind, ValidatedBlocklist, decode, encode,
};
use keitora_service::engine::{EventSink, ServiceEngine};
use keitora_service::pipe::{PipeServer, RequestHandler};
use keitora_service::store::{
    BlockEvent as StoredBlockEvent, Store, TargetKind as StoredTargetKind,
};

use self::fake::TestClock;

/// The id of the blocklist most tests start a session with.
const WORK: &str = "work";

/// The id of a second blocklist every test store holds.
const PLAY: &str = "play";

/// The test clock's wall reading when a test starts, in seconds since the
/// Unix epoch (2027-01-15T08:00:00Z).
const WALL_START_SECS: u64 = 1_800_000_000;

/// How long a test waits for the pipe or a process before failing.
const WAIT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// The test clock
// ---------------------------------------------------------------------------

/// The only place in this file that names `FakeClock`, which the crate's
/// `clippy.toml` bans outside tests.
#[expect(
    clippy::disallowed_types,
    reason = "a test needs a clock it moves by hand; production code never uses FakeClock"
)]
mod fake {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use keitora_core::clock::{Clock, FakeClock, MonotonicInstant};

    /// A clock the test moves by hand. Clones share the same time.
    #[derive(Debug, Clone)]
    pub(super) struct TestClock(FakeClock);

    impl TestClock {
        /// A clock whose wall reading is `wall`.
        pub(super) fn new(wall: SystemTime) -> Self {
            Self(FakeClock::new(wall))
        }

        /// Lets `by` pass, on both readings.
        pub(super) fn advance(&self, by: Duration) {
            self.0.advance(by).expect("the test clock advances");
        }

        /// Sets the wall reading only, as when the user changes the date.
        pub(super) fn set_wall(&self, wall: SystemTime) {
            self.0.set_wall(wall);
        }

        /// A handle on this clock's time, for the engine.
        pub(super) fn shared(&self) -> Arc<dyn Clock + Send + Sync> {
            Arc::new(self.0.clone())
        }
    }

    impl Clock for TestClock {
        fn monotonic(&self) -> MonotonicInstant {
            self.0.monotonic()
        }

        fn wall(&self) -> SystemTime {
            self.0.wall()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers: data, events and the engine
// ---------------------------------------------------------------------------

/// A fresh, empty data folder for one test, named after it, under Cargo's
/// temporary folder for integration tests.
fn data_dir(test_name: &str) -> PathBuf {
    // Before any code under test can log (see `install_log_capture`).
    install_log_capture();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("acceptance_t2_5")
        .join(test_name);
    match fs::remove_dir_all(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => panic!("couldn't clear the sandbox {}: {error}", root.display()),
    }
    let dir = root.join("data");
    fs::create_dir_all(&dir).expect("couldn't create the sandbox");
    dir
}

/// The wall time every test clock starts at.
fn wall_start() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(WALL_START_SECS))
        .expect("a valid wall time")
}

/// `WALL_START_SECS` in milliseconds.
fn wall_start_ms() -> u64 {
    WALL_START_SECS.checked_mul(1000).expect("in range")
}

/// A valid preset.
fn preset(
    focus_min: u32,
    short_break_min: u32,
    long_break_min: u32,
    cycles: u32,
) -> PomodoroPreset {
    PomodoroPreset::new(focus_min, short_break_min, long_break_min, cycles)
        .expect("test setup: a valid preset")
}

/// A valid blocklist with one site target.
fn blocklist(id: &str, name: &str, site: &str) -> Blocklist {
    let blocklist = Blocklist {
        id: id.to_owned(),
        name: name.to_owned(),
        color: "#336699".to_owned(),
        targets: vec![Target::Site {
            pattern: SitePattern::new(site),
        }],
        exceptions: Vec::new(),
        block_during_breaks: false,
    };
    blocklist.validate().expect("test setup: a valid blocklist");
    blocklist
}

/// `blocklist` as a request carries it.
fn validated(blocklist: Blocklist) -> ValidatedBlocklist {
    ValidatedBlocklist::new(blocklist).expect("test setup: a valid blocklist")
}

/// The blocklist `WORK` as every test store first holds it.
fn work_list() -> Blocklist {
    blocklist(WORK, "Work", "news.example.com")
}

/// The blocklist `PLAY` as every test store first holds it.
fn play_list() -> Blocklist {
    blocklist(PLAY, "Play", "games.example.com")
}

/// A page request.
fn page(offset: u32, limit: u32) -> PageRequest {
    PageRequest::new(offset, limit).expect("test setup: a valid page")
}

/// `secs` in milliseconds.
fn ms(secs: u64) -> u64 {
    secs.checked_mul(1000).expect("in range")
}

/// The state of a running, unpaused session.
fn running(
    phase: Phase,
    cycle: u32,
    remaining_secs: u64,
    lock: LockKind,
    blocklist_id: &str,
) -> SessionState {
    SessionState {
        phase,
        cycle,
        remaining_ms: ms(remaining_secs),
        paused: false,
        lock: Some(lock),
        blocklist_id: Some(blocklist_id.to_owned()),
    }
}

/// Fails the test unless `state` is the state of an engine with no session.
/// The cycle of an idle state isn't checked.
fn assert_idle(state: &SessionState, what: &str) {
    assert_eq!(state.phase, Phase::Idle, "{what}: {state:?}");
    assert_eq!(state.remaining_ms, 0, "{what}: {state:?}");
    assert!(!state.paused, "{what}: {state:?}");
    assert_eq!(state.lock, None, "{what}: {state:?}");
    assert_eq!(state.blocklist_id, None, "{what}: {state:?}");
}

/// The refusal of a strict session.
fn strict_locked() -> Response {
    Response::Error {
        error: ProtocolError::StrictSessionLocked {
            reason: LockReason::UntilTimerEnds,
        },
    }
}

/// An error response with `error`.
fn error(error: ProtocolError) -> Response {
    Response::Error { error }
}

/// Records every event the engine publishes.
#[derive(Debug, Default)]
struct Recorder {
    events: Mutex<Vec<Event>>,
}

impl Recorder {
    /// The events published since the last call, in order.
    fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl EventSink for Recorder {
    fn publish(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

/// An engine on a test store, with its clock and its published events.
struct Harness {
    dir: PathBuf,
    clock: TestClock,
    events: Arc<Recorder>,
    engine: Arc<ServiceEngine>,
}

/// An engine on a fresh store holding the blocklists `WORK` and `PLAY`.
fn harness(test_name: &str) -> Harness {
    harness_with(test_name, |_| {})
}

/// Like [`harness`], with `prepare` run on the store before the engine is
/// created.
fn harness_with(test_name: &str, prepare: impl FnOnce(&mut Store)) -> Harness {
    let dir = data_dir(test_name);
    let mut store = Store::open(&dir).expect("test setup: the store opens");
    store
        .put_blocklist(&work_list())
        .expect("test setup: stores a blocklist");
    store
        .put_blocklist(&play_list())
        .expect("test setup: stores a blocklist");
    prepare(&mut store);
    let clock = TestClock::new(wall_start());
    let events = Arc::new(Recorder::default());
    let sink: Arc<dyn EventSink> = events.clone();
    let engine = ServiceEngine::new(store, clock.shared(), sink).expect("the engine starts");
    Harness {
        dir,
        clock,
        events,
        engine: Arc::new(engine),
    }
}

impl Harness {
    /// The engine's answer to `request`.
    fn call(&self, request: Request) -> Response {
        self.engine.handle(request)
    }

    /// Fails the test unless `request` is answered `Ok`.
    fn ok(&self, request: Request) {
        let what = format!("{request:?}");
        assert_eq!(self.call(request), Response::Ok, "{what} should succeed");
    }

    /// The state `GetState` reports.
    fn state(&self) -> SessionState {
        match self.call(Request::GetState) {
            Response::State { state } => state,
            other => panic!("GetState should answer State, got {other:?}"),
        }
    }

    /// The answer to a `StartSession`.
    fn start(&self, blocklist_id: &str, preset: PomodoroPreset, lock: LockKind) -> Response {
        self.call(Request::StartSession {
            blocklist_id: blocklist_id.to_owned(),
            preset,
            lock,
        })
    }

    /// Lets `secs` seconds pass on the test clock.
    fn advance_secs(&self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
    }

    /// The events published since the last call.
    fn take_events(&self) -> Vec<Event> {
        self.events.take()
    }

    /// The whole history, as `GetHistory` reports it.
    fn history(&self) -> Vec<HistoryEntry> {
        match self.call(Request::GetHistory {
            range: HistoryRange::All,
            page: page(0, 100),
        }) {
            Response::History { items, .. } => items,
            other => panic!("GetHistory should answer History, got {other:?}"),
        }
    }

    /// The blocklist `id` as `GetBlocklist` reports it, or `None` when it
    /// answers `not_found`.
    fn get_blocklist(&self, id: &str) -> Option<Blocklist> {
        match self.call(Request::GetBlocklist { id: id.to_owned() }) {
            Response::Blocklist { blocklist } => Some(blocklist.into_blocklist()),
            Response::Error {
                error: ProtocolError::NotFound,
            } => None,
            other => panic!("GetBlocklist {id} answered {other:?}"),
        }
    }

    /// A second, independent view of the store on disk.
    fn store_on_disk(&self) -> Store {
        Store::open(&self.dir).expect("the store opens a second time")
    }
}

/// The `StateChanged` events in `events`, in order.
fn state_changes(events: &[Event]) -> Vec<SessionState> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::StateChanged { state } => Some(state.clone()),
            _ => None,
        })
        .collect()
}

/// The `SessionEnded` outcomes in `events`, in order.
fn session_ends(events: &[Event]) -> Vec<Outcome> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::SessionEnded { outcome } => Some(*outcome),
            _ => None,
        })
        .collect()
}

/// The history entry of a session that started at the test clock's start.
fn history_entry(
    preset: PomodoroPreset,
    lock: LockKind,
    outcome: Outcome,
    focus_secs: u64,
) -> HistoryEntry {
    HistoryEntry {
        started_at_unix_ms: wall_start_ms(),
        blocklist_id: WORK.to_owned(),
        preset,
        lock,
        outcome,
        focus_ms: ms(focus_secs),
    }
}

// ---------------------------------------------------------------------------
// Helpers: logs
// ---------------------------------------------------------------------------

/// Where captured log lines are written.
#[derive(Debug, Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

thread_local! {
    /// The buffer [`capture_logs`] is filling on this thread, if any.
    static CAPTURE: RefCell<Option<LogBuffer>> = const { RefCell::new(None) };
}

/// Writes each record to the capture buffer of the thread that logged it,
/// or nowhere if that thread isn't capturing.
#[derive(Debug)]
struct ThisThreadsCapture;

impl io::Write for ThisThreadsCapture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match CAPTURE.with_borrow(Clone::clone) {
            Some(mut buffer) => io::Write::write(&mut buffer, bytes),
            None => Ok(bytes.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Installs, once per test binary, one global subscriber that formats every
/// record, at every level, into [`ThisThreadsCapture`].
///
/// `tracing` caches whether each callsite is enabled globally, for every
/// thread. With a subscriber per test (`with_default`), a callsite first hit
/// while another test's subscriber was being registered could cache
/// "disabled", and a later capture would miss its records. With one global
/// subscriber at `TRACE`, installed before any test runs code that logs,
/// every callsite is always enabled; which thread's buffer a record goes to
/// is decided when it is written.
fn install_log_capture() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(|| ThisThreadsCapture)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other global subscriber is set in this test binary");
    });
}

/// Runs `action` with every `tracing` event of this thread, at every level,
/// written to a buffer, and returns its result and the text logged. Records
/// from other threads aren't captured.
fn capture_logs<T>(action: impl FnOnce() -> T) -> (T, String) {
    install_log_capture();
    let buffer = LogBuffer::default();
    let previous = CAPTURE.replace(Some(buffer.clone()));
    let result = action();
    CAPTURE.set(previous);
    let bytes = buffer
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (result, String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// EN1 — time comes from the injected clock
// ---------------------------------------------------------------------------

#[test]
fn get_state_reports_idle_before_any_session() {
    let h = harness("get_state_reports_idle_before_any_session");
    assert_idle(&h.state(), "a new engine");
}

#[test]
fn remaining_time_comes_from_the_injected_monotonic_clock() {
    let h = harness("remaining_time_comes_from_the_injected_monotonic_clock");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );

    h.advance_secs(10 * 60);
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 15 * 60, LockKind::None, WORK),
        "10 minutes on the injected clock leave 15 of 25"
    );

    // Changing the date, forwards or backwards, moves nothing.
    h.clock.set_wall(UNIX_EPOCH);
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 15 * 60, LockKind::None, WORK)
    );
    h.clock
        .set_wall(wall_start() + Duration::from_secs(365 * 24 * 60 * 60));
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 15 * 60, LockKind::None, WORK)
    );
}

#[test]
fn history_takes_the_start_time_from_the_injected_wall_clock() {
    let h = harness("history_takes_the_start_time_from_the_injected_wall_clock");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(60);
    h.ok(Request::StopSession { text: None });

    let history = h.history();
    assert_eq!(history.len(), 1, "{history:?}");
    assert_eq!(
        history[0].started_at_unix_ms,
        WALL_START_SECS * 1000,
        "the start time is the injected clock's wall reading, not the system's"
    );
}

// ---------------------------------------------------------------------------
// EN2 — session requests
// ---------------------------------------------------------------------------

#[test]
fn start_session_runs_the_first_focus_of_the_requested_preset_and_lock() {
    let h = harness("start_session_runs_the_first_focus_of_the_requested_preset_and_lock");
    assert_eq!(
        h.start(WORK, preset(50, 10, 20, 2), LockKind::UntilTimerEnds),
        Response::Ok
    );
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 50 * 60, LockKind::UntilTimerEnds, WORK)
    );
}

#[test]
fn phases_follow_the_clock() {
    let h = harness("phases_follow_the_clock");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 2), LockKind::None),
        Response::Ok
    );

    h.advance_secs(25 * 60);
    assert_eq!(
        h.state(),
        running(Phase::ShortBreak, 1, 5 * 60, LockKind::None, WORK)
    );
    h.advance_secs(5 * 60);
    assert_eq!(
        h.state(),
        running(Phase::Focus, 2, 25 * 60, LockKind::None, WORK)
    );
    h.advance_secs(25 * 60);
    assert_eq!(
        h.state(),
        running(Phase::LongBreak, 2, 15 * 60, LockKind::None, WORK)
    );
    h.advance_secs(15 * 60);
    assert_idle(&h.state(), "after the long break");
}

#[test]
fn start_session_with_an_unknown_blocklist_is_not_found_and_starts_nothing() {
    let h = harness("start_session_with_an_unknown_blocklist_is_not_found_and_starts_nothing");
    assert_eq!(
        h.start("missing", preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        error(ProtocolError::NotFound)
    );
    assert_idle(&h.state(), "after a refused start");
    assert_eq!(h.take_events(), Vec::new());
}

#[test]
fn blocklist_ids_are_compared_exactly() {
    let h = harness("blocklist_ids_are_compared_exactly");
    assert_eq!(
        h.start("Work", preset(25, 5, 15, 4), LockKind::None),
        error(ProtocolError::NotFound),
        "`Work` is not `work`"
    );
    assert_idle(&h.state(), "after a refused start");
    assert_eq!(h.get_blocklist("WORK"), None);
    assert_eq!(h.get_blocklist(WORK), Some(work_list()));
}

#[test]
fn start_session_over_a_running_unlocked_session_is_invalid_state() {
    let h = harness("start_session_over_a_running_unlocked_session_is_invalid_state");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(60);
    assert_eq!(
        h.start(PLAY, preset(50, 10, 20, 2), LockKind::None),
        error(ProtocolError::InvalidState)
    );
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 24 * 60, LockKind::None, WORK),
        "the running session is unchanged"
    );
}

#[test]
fn pause_and_resume_freeze_and_continue_the_timer() {
    let h = harness("pause_and_resume_freeze_and_continue_the_timer");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(10 * 60);

    h.ok(Request::PauseSession);
    let paused = SessionState {
        paused: true,
        ..running(Phase::Focus, 1, 15 * 60, LockKind::None, WORK)
    };
    assert_eq!(h.state(), paused);
    h.advance_secs(30 * 60);
    assert_eq!(h.state(), paused, "a paused session doesn't move");

    h.ok(Request::ResumeSession);
    h.advance_secs(5 * 60);
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 10 * 60, LockKind::None, WORK)
    );
}

#[test]
fn session_requests_without_a_session_are_invalid_state() {
    let h = harness("session_requests_without_a_session_are_invalid_state");
    for request in [
        Request::PauseSession,
        Request::ResumeSession,
        Request::SkipBreak,
        Request::StopSession { text: None },
    ] {
        let what = format!("{request:?}");
        assert_eq!(
            h.call(request),
            error(ProtocolError::InvalidState),
            "{what} with no session"
        );
    }
    assert_idle(&h.state(), "still");
}

#[test]
fn a_second_pause_or_a_resume_while_running_is_invalid_state() {
    let h = harness("a_second_pause_or_a_resume_while_running_is_invalid_state");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    assert_eq!(
        h.call(Request::ResumeSession),
        error(ProtocolError::InvalidState),
        "resume while running"
    );
    h.ok(Request::PauseSession);
    assert_eq!(
        h.call(Request::PauseSession),
        error(ProtocolError::InvalidState),
        "pause while paused"
    );
    assert!(h.state().paused);
}

#[test]
fn skip_break_during_a_focus_is_invalid_state() {
    let h = harness("skip_break_during_a_focus_is_invalid_state");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(60);
    assert_eq!(
        h.call(Request::SkipBreak),
        error(ProtocolError::InvalidState)
    );
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 24 * 60, LockKind::None, WORK)
    );
}

#[test]
fn skip_break_during_a_short_break_starts_the_next_focus() {
    let h = harness("skip_break_during_a_short_break_starts_the_next_focus");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(26 * 60);
    h.ok(Request::SkipBreak);
    assert_eq!(
        h.state(),
        running(Phase::Focus, 2, 25 * 60, LockKind::None, WORK)
    );
}

#[test]
fn stop_ends_an_unlocked_session() {
    let h = harness("stop_ends_an_unlocked_session");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(60);
    h.ok(Request::StopSession { text: None });
    assert_idle(&h.state(), "after a stop");
}

// ---------------------------------------------------------------------------
// EN3 — strict sessions
// ---------------------------------------------------------------------------

#[test]
fn a_strict_session_refuses_stop_pause_skip_and_another_start() {
    let h = harness("a_strict_session_refuses_stop_pause_skip_and_another_start");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );
    h.advance_secs(60);
    let focus = running(Phase::Focus, 1, 24 * 60, LockKind::UntilTimerEnds, WORK);

    for request in [
        Request::StopSession { text: None },
        Request::StopSession {
            text: Some("anything".to_owned()),
        },
        Request::PauseSession,
        Request::StartSession {
            blocklist_id: PLAY.to_owned(),
            preset: preset(1, 1, 1, 1),
            lock: LockKind::None,
        },
        Request::StartSession {
            blocklist_id: WORK.to_owned(),
            preset: preset(1, 1, 1, 1),
            lock: LockKind::UntilTimerEnds,
        },
    ] {
        let what = format!("{request:?}");
        assert_eq!(
            h.call(request),
            strict_locked(),
            "{what} during a strict focus"
        );
        assert_eq!(h.state(), focus, "{what} changed the strict session");
    }

    // In the short break, which a strict session can't skip either.
    h.advance_secs(25 * 60);
    let short_break = running(Phase::ShortBreak, 1, 4 * 60, LockKind::UntilTimerEnds, WORK);
    assert_eq!(h.state(), short_break);
    assert_eq!(h.call(Request::SkipBreak), strict_locked());
    assert_eq!(h.call(Request::StopSession { text: None }), strict_locked());
    assert_eq!(h.call(Request::PauseSession), strict_locked());
    assert_eq!(h.state(), short_break, "the break goes on, whole");
}

#[test]
fn a_strict_session_refuses_editing_or_deleting_its_blocklist() {
    let h = harness("a_strict_session_refuses_editing_or_deleting_its_blocklist");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );

    let emptied = Blocklist {
        targets: Vec::new(),
        ..work_list()
    };
    let with_exception = Blocklist {
        exceptions: vec![SitePattern::new("news.example.com")],
        ..work_list()
    };
    for edited in [emptied, with_exception] {
        assert_eq!(
            h.call(Request::UpsertBlocklist {
                blocklist: validated(edited),
            }),
            strict_locked()
        );
    }
    assert_eq!(
        h.call(Request::DeleteBlocklist {
            id: WORK.to_owned()
        }),
        strict_locked()
    );

    assert_eq!(h.get_blocklist(WORK), Some(work_list()));
    assert_eq!(
        h.store_on_disk()
            .blocklist(WORK)
            .expect("the store reads the blocklist"),
        Some(work_list()),
        "the stored blocklist is unchanged"
    );
}

#[test]
fn a_strict_session_refuses_editing_deleting_or_creating_any_other_blocklist() {
    let h = harness("a_strict_session_refuses_editing_deleting_or_creating_any_other_blocklist");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );

    assert_eq!(
        h.call(Request::UpsertBlocklist {
            blocklist: validated(blocklist(PLAY, "Play", "other.example.com")),
        }),
        strict_locked(),
        "editing another blocklist"
    );
    assert_eq!(
        h.call(Request::DeleteBlocklist {
            id: PLAY.to_owned()
        }),
        strict_locked(),
        "deleting another blocklist"
    );
    assert_eq!(
        h.call(Request::UpsertBlocklist {
            blocklist: validated(blocklist("new-list", "New", "new.example.com")),
        }),
        strict_locked(),
        "creating a blocklist"
    );

    let store = h.store_on_disk();
    assert_eq!(
        store.blocklists().expect("the store lists blocklists"),
        vec![work_list(), play_list()],
        "nothing was stored or deleted"
    );
}

#[test]
fn updating_settings_during_a_strict_session_doesnt_touch_it() {
    let h = harness("updating_settings_during_a_strict_session_doesnt_touch_it");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );
    h.advance_secs(5 * 60);
    let before = h.state();
    h.take_events();

    let shortest = Settings {
        default_preset: preset(1, 1, 1, 1),
    };
    h.ok(Request::UpdateSettings {
        settings: shortest.clone(),
    });

    assert_eq!(
        h.state(),
        before,
        "the running strict session keeps its preset and time"
    );
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 20 * 60, LockKind::UntilTimerEnds, WORK)
    );
    assert_eq!(
        h.call(Request::GetSettings),
        Response::Settings { settings: shortest },
        "the settings themselves were updated"
    );
    assert_eq!(state_changes(&h.take_events()), Vec::new());
    assert_eq!(
        h.call(Request::StopSession { text: None }),
        strict_locked(),
        "still strict"
    );
}

#[test]
fn a_strict_session_unlocks_everything_once_its_timer_ends() {
    let h = harness("a_strict_session_unlocks_everything_once_its_timer_ends");
    assert_eq!(
        h.start(WORK, preset(1, 1, 1, 1), LockKind::UntilTimerEnds),
        Response::Ok
    );
    h.advance_secs(2 * 60);
    assert_idle(&h.state(), "after the strict session's last phase");

    assert_eq!(
        h.call(Request::StopSession { text: None }),
        error(ProtocolError::InvalidState),
        "nothing left to stop"
    );
    let edited = blocklist(WORK, "Work", "edited.example.com");
    h.ok(Request::UpsertBlocklist {
        blocklist: validated(edited.clone()),
    });
    assert_eq!(h.get_blocklist(WORK), Some(edited));
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );
}

#[test]
fn an_unlocked_session_allows_editing_its_blocklist() {
    let h = harness("an_unlocked_session_allows_editing_its_blocklist");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    let edited = blocklist(WORK, "Work", "edited.example.com");
    h.ok(Request::UpsertBlocklist {
        blocklist: validated(edited.clone()),
    });
    assert_eq!(h.get_blocklist(WORK), Some(edited));
    h.ok(Request::DeleteBlocklist {
        id: PLAY.to_owned(),
    });
    assert_eq!(h.get_blocklist(PLAY), None);
}

// ---------------------------------------------------------------------------
// EN4 — RandomText sessions
// ---------------------------------------------------------------------------

#[test]
fn a_random_text_session_refuses_a_stop_without_its_text() {
    let h = harness("a_random_text_session_refuses_a_stop_without_its_text");
    let lock = LockKind::RandomText { len: 12 };
    assert_eq!(h.start(WORK, preset(25, 5, 15, 4), lock), Response::Ok);
    h.advance_secs(60);
    h.take_events();

    for text in [
        None,
        Some(String::new()),
        Some("aaaaaaaaaaaa".to_owned()),
        Some("too-short".to_owned()),
        Some("much-too-long-for-twelve".to_owned()),
    ] {
        let what = format!("{text:?}");
        match h.call(Request::StopSession { text }) {
            Response::Error {
                error: ProtocolError::Validation { field, .. },
            } => assert_eq!(field, "text", "stop with {what}"),
            other => {
                panic!("stop with {what} should be a validation error on `text`, got {other:?}")
            }
        }
        assert_eq!(
            h.state(),
            running(Phase::Focus, 1, 24 * 60, lock, WORK),
            "the session goes on after stop with {what}"
        );
    }
    assert_eq!(h.take_events(), Vec::new(), "refused stops publish nothing");
    assert_eq!(h.history(), Vec::new(), "nothing ended");
}

#[test]
fn the_typed_stop_text_is_never_logged() {
    let h = harness("the_typed_stop_text_is_never_logged");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::RandomText { len: 25 }),
        Response::Ok
    );
    let typed = "Zq7TypedTextNeverLogged42";
    let (response, logs) = capture_logs(|| {
        h.call(Request::StopSession {
            text: Some(typed.to_owned()),
        })
    });
    assert!(
        matches!(response, Response::Error { .. }),
        "a wrong text is refused: {response:?}"
    );
    assert!(
        !logs.contains(typed),
        "the typed text appears in the logs:\n{logs}"
    );
}

#[test]
fn a_random_text_session_can_be_paused_and_its_break_skipped_without_text() {
    let h = harness("a_random_text_session_can_be_paused_and_its_break_skipped_without_text");
    let lock = LockKind::RandomText { len: 8 };
    assert_eq!(h.start(WORK, preset(25, 5, 15, 4), lock), Response::Ok);
    h.ok(Request::PauseSession);
    assert!(h.state().paused);
    h.ok(Request::ResumeSession);
    h.advance_secs(26 * 60);
    h.ok(Request::SkipBreak);
    assert_eq!(h.state(), running(Phase::Focus, 2, 25 * 60, lock, WORK));
}

// ---------------------------------------------------------------------------
// EN5 — events
// ---------------------------------------------------------------------------

#[test]
fn starting_a_session_publishes_its_new_state() {
    let h = harness("starting_a_session_publishes_its_new_state");
    h.take_events();
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );
    assert_eq!(
        h.take_events(),
        vec![Event::StateChanged {
            state: running(Phase::Focus, 1, 25 * 60, LockKind::UntilTimerEnds, WORK),
        }]
    );
}

#[test]
fn pause_resume_and_skip_publish_the_new_state() {
    let h = harness("pause_resume_and_skip_publish_the_new_state");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(10 * 60);
    h.take_events();

    h.ok(Request::PauseSession);
    let paused = SessionState {
        paused: true,
        ..running(Phase::Focus, 1, 15 * 60, LockKind::None, WORK)
    };
    assert_eq!(h.take_events(), vec![Event::StateChanged { state: paused }]);

    h.ok(Request::ResumeSession);
    assert_eq!(
        h.take_events(),
        vec![Event::StateChanged {
            state: running(Phase::Focus, 1, 15 * 60, LockKind::None, WORK),
        }]
    );

    h.advance_secs(16 * 60);
    h.engine.tick();
    h.take_events();
    h.ok(Request::SkipBreak);
    assert_eq!(
        h.take_events(),
        vec![Event::StateChanged {
            state: running(Phase::Focus, 2, 25 * 60, LockKind::None, WORK),
        }]
    );
}

#[test]
fn stopping_publishes_session_ended_and_the_idle_state() {
    let h = harness("stopping_publishes_session_ended_and_the_idle_state");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(60);
    h.take_events();

    h.ok(Request::StopSession { text: None });
    let events = h.take_events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(session_ends(&events), vec![Outcome::Stopped], "{events:?}");
    let changes = state_changes(&events);
    assert_eq!(changes.len(), 1, "{events:?}");
    assert_idle(&changes[0], "the state published after a stop");
}

#[test]
fn a_tick_publishes_each_phase_change() {
    let h = harness("a_tick_publishes_each_phase_change");
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.take_events();

    h.advance_secs(25 * 60);
    h.engine.tick();
    assert_eq!(
        h.take_events(),
        vec![Event::StateChanged {
            state: running(Phase::ShortBreak, 1, 5 * 60, LockKind::None, WORK),
        }]
    );

    h.advance_secs(5 * 60);
    h.engine.tick();
    assert_eq!(
        h.take_events(),
        vec![Event::StateChanged {
            state: running(Phase::Focus, 2, 25 * 60, LockKind::None, WORK),
        }]
    );
}

#[test]
fn a_tick_without_a_transition_publishes_nothing() {
    let h = harness("a_tick_without_a_transition_publishes_nothing");
    h.engine.tick();
    assert_eq!(h.take_events(), Vec::new(), "an idle tick");

    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );
    h.take_events();
    for _ in 0..3 {
        h.advance_secs(60);
        h.engine.tick();
    }
    assert_eq!(h.take_events(), Vec::new(), "ticks within one focus");
}

#[test]
fn refused_requests_publish_nothing() {
    let h = harness("refused_requests_publish_nothing");
    assert_eq!(
        h.start("missing", preset(25, 5, 15, 4), LockKind::None),
        error(ProtocolError::NotFound)
    );
    assert_eq!(
        h.call(Request::PauseSession),
        error(ProtocolError::InvalidState)
    );
    assert_eq!(
        h.call(Request::DeleteBlocklist {
            id: "missing".to_owned(),
        }),
        error(ProtocolError::NotFound)
    );
    assert_eq!(h.take_events(), Vec::new(), "refused while idle");

    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::UntilTimerEnds),
        Response::Ok
    );
    h.take_events();
    assert_eq!(h.call(Request::StopSession { text: None }), strict_locked());
    assert_eq!(h.call(Request::PauseSession), strict_locked());
    assert_eq!(
        h.call(Request::UpsertBlocklist {
            blocklist: validated(blocklist(WORK, "Work", "x.example.com")),
        }),
        strict_locked()
    );
    assert_eq!(
        h.call(Request::DeleteBlocklist {
            id: WORK.to_owned()
        }),
        strict_locked()
    );
    assert_eq!(
        h.take_events(),
        Vec::new(),
        "refused during a strict session"
    );
}

#[test]
fn blocklist_edits_publish_blocklist_changed() {
    let h = harness("blocklist_edits_publish_blocklist_changed");
    h.take_events();

    h.ok(Request::UpsertBlocklist {
        blocklist: validated(blocklist("reading", "Reading", "books.example.com")),
    });
    assert_eq!(
        h.take_events(),
        vec![Event::BlocklistChanged {
            id: "reading".to_owned(),
        }],
        "a new blocklist"
    );

    h.ok(Request::UpsertBlocklist {
        blocklist: validated(blocklist(WORK, "Work", "edited.example.com")),
    });
    assert_eq!(
        h.take_events(),
        vec![Event::BlocklistChanged {
            id: WORK.to_owned()
        }],
        "a changed blocklist"
    );

    h.ok(Request::DeleteBlocklist {
        id: PLAY.to_owned(),
    });
    assert_eq!(
        h.take_events(),
        vec![Event::BlocklistChanged {
            id: PLAY.to_owned()
        }],
        "a deleted blocklist"
    );
}

// ---------------------------------------------------------------------------
// EN6 — the end of a session
// ---------------------------------------------------------------------------

#[test]
fn a_session_that_runs_out_publishes_session_ended_and_is_recorded_as_completed() {
    let h = harness("a_session_that_runs_out_publishes_session_ended_and_is_recorded_as_completed");
    let shortest = preset(1, 1, 1, 1);
    assert_eq!(
        h.start(WORK, shortest.clone(), LockKind::UntilTimerEnds),
        Response::Ok
    );
    h.take_events();

    h.advance_secs(60);
    h.engine.tick();
    assert_eq!(
        h.take_events(),
        vec![Event::StateChanged {
            state: running(Phase::LongBreak, 1, 60, LockKind::UntilTimerEnds, WORK),
        }]
    );
    assert_eq!(h.history(), Vec::new(), "not over yet");

    h.advance_secs(60);
    h.engine.tick();
    let events = h.take_events();
    assert_eq!(
        session_ends(&events),
        vec![Outcome::Completed],
        "{events:?}"
    );
    let changes = state_changes(&events);
    assert_eq!(changes.len(), 1, "{events:?}");
    assert_idle(&changes[0], "the state published at the end");
    assert_eq!(events.len(), 2, "{events:?}");

    assert_eq!(
        h.history(),
        vec![history_entry(
            shortest,
            LockKind::UntilTimerEnds,
            Outcome::Completed,
            60
        )]
    );

    h.advance_secs(60);
    h.engine.tick();
    assert_eq!(h.take_events(), Vec::new(), "the end is published once");
    assert_eq!(h.history().len(), 1, "and recorded once");
}

#[test]
fn a_stopped_session_is_recorded_as_stopped_with_its_focus_time() {
    let h = harness("a_stopped_session_is_recorded_as_stopped_with_its_focus_time");
    let lock = LockKind::None;
    assert_eq!(h.start(WORK, PomodoroPreset::default(), lock), Response::Ok);
    h.advance_secs(10 * 60);
    h.ok(Request::StopSession { text: None });

    assert_eq!(
        h.history(),
        vec![history_entry(
            PomodoroPreset::default(),
            lock,
            Outcome::Stopped,
            10 * 60
        )]
    );
}

#[test]
fn a_session_that_ended_between_ticks_is_reported_once_before_the_next_start() {
    let h = harness("a_session_that_ended_between_ticks_is_reported_once_before_the_next_start");
    let shortest = preset(1, 1, 1, 1);
    assert_eq!(
        h.start(WORK, shortest.clone(), LockKind::None),
        Response::Ok
    );
    h.take_events();

    // It runs out with no tick in between, and a new session is started.
    h.advance_secs(2 * 60);
    assert_eq!(
        h.start(PLAY, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );

    let events = h.take_events();
    assert_eq!(
        session_ends(&events),
        vec![Outcome::Completed],
        "{events:?}"
    );
    let ended_at = events
        .iter()
        .position(|event| matches!(event, Event::SessionEnded { .. }))
        .expect("SessionEnded was published");
    let new_focus = Event::StateChanged {
        state: running(Phase::Focus, 1, 25 * 60, LockKind::None, PLAY),
    };
    let started_at = events
        .iter()
        .position(|event| *event == new_focus)
        .expect("the new session's state was published");
    assert!(
        ended_at < started_at,
        "the old session's end comes before the new start: {events:?}"
    );
    assert_eq!(
        h.history(),
        vec![history_entry(
            shortest,
            LockKind::None,
            Outcome::Completed,
            60
        )]
    );

    h.engine.tick();
    assert_eq!(
        session_ends(&h.take_events()),
        Vec::new(),
        "the end is published once"
    );
    assert_eq!(h.history().len(), 1, "and recorded once");
}

// ---------------------------------------------------------------------------
// EN7 — blocklists, settings, history and block events
// ---------------------------------------------------------------------------

#[test]
fn blocklists_can_be_created_read_and_deleted() {
    let h = harness("blocklists_can_be_created_read_and_deleted");
    let reading = blocklist("reading", "Reading", "books.example.com");
    h.ok(Request::UpsertBlocklist {
        blocklist: validated(reading.clone()),
    });
    assert_eq!(h.get_blocklist("reading"), Some(reading.clone()));
    assert_eq!(
        h.store_on_disk()
            .blocklist("reading")
            .expect("the store reads it"),
        Some(reading),
        "the new blocklist is in the store"
    );

    h.ok(Request::DeleteBlocklist {
        id: "reading".to_owned(),
    });
    assert_eq!(h.get_blocklist("reading"), None);
    assert_eq!(
        h.store_on_disk()
            .blocklist("reading")
            .expect("the store reads it"),
        None,
        "the blocklist is gone from the store"
    );
}

#[test]
fn deleting_an_unknown_blocklist_is_not_found() {
    let h = harness("deleting_an_unknown_blocklist_is_not_found");
    assert_eq!(
        h.call(Request::DeleteBlocklist {
            id: "missing".to_owned(),
        }),
        error(ProtocolError::NotFound)
    );
}

/// The summary `ListBlocklists` gives for `blocklist`.
fn summary(blocklist: &Blocklist) -> BlocklistSummary {
    BlocklistSummary {
        id: blocklist.id.clone(),
        name: blocklist.name.clone(),
        color: blocklist.color.clone(),
    }
}

#[test]
fn list_blocklists_pages_in_the_stored_order() {
    let extra: Vec<Blocklist> = (1..=5)
        .map(|n| blocklist(&format!("list-{n}"), &format!("List {n}"), "example.com"))
        .collect();
    let to_store = extra.clone();
    let h = harness_with("list_blocklists_pages_in_the_stored_order", move |store| {
        for list in &to_store {
            store
                .put_blocklist(list)
                .expect("test setup: stores a blocklist");
        }
    });
    let mut all = vec![summary(&work_list()), summary(&play_list())];
    all.extend(extra.iter().map(summary));

    let list = |offset: u32, limit: u32| match h.call(Request::ListBlocklists {
        page: page(offset, limit),
    }) {
        Response::Blocklists { items, next_offset } => (items, next_offset),
        other => panic!("ListBlocklists should answer Blocklists, got {other:?}"),
    };

    assert_eq!(list(0, 3), (all[0..3].to_vec(), Some(3)));
    assert_eq!(list(3, 3), (all[3..6].to_vec(), Some(6)));
    assert_eq!(list(6, 3), (all[6..7].to_vec(), None));
    assert_eq!(
        list(5, 2),
        (all[5..7].to_vec(), None),
        "a page that ends the list"
    );
    assert_eq!(list(0, 100), (all.clone(), None));
    assert_eq!(list(7, 10), (Vec::new(), None), "just past the end");
}

#[test]
fn paged_requests_with_a_huge_offset_return_an_empty_last_page() {
    let h = harness_with(
        "paged_requests_with_a_huge_offset_return_an_empty_last_page",
        |store| {
            store
                .record_block_event(&StoredBlockEvent {
                    at: wall_start(),
                    kind: StoredTargetKind::App,
                    target: "game.exe".to_owned(),
                })
                .expect("test setup: records a block event");
        },
    );
    // One finished session, so the history isn't empty.
    assert_eq!(
        h.start(WORK, preset(25, 5, 15, 4), LockKind::None),
        Response::Ok
    );
    h.advance_secs(60);
    h.ok(Request::StopSession { text: None });

    for offset in [u32::MAX, u32::MAX - 1, u32::MAX - 99, u32::MAX / 2 + 1] {
        assert_eq!(
            h.call(Request::ListBlocklists {
                page: page(offset, 100),
            }),
            Response::Blocklists {
                items: Vec::new(),
                next_offset: None,
            },
            "ListBlocklists at offset {offset}"
        );
        for range in [HistoryRange::All, HistoryRange::Today] {
            assert_eq!(
                h.call(Request::GetHistory {
                    range,
                    page: page(offset, 100),
                }),
                Response::History {
                    items: Vec::new(),
                    next_offset: None,
                },
                "GetHistory {range:?} at offset {offset}"
            );
        }
        assert_eq!(
            h.call(Request::GetBlockEvents {
                page: page(offset, 100),
            }),
            Response::BlockEvents {
                items: Vec::new(),
                next_offset: None,
            },
            "GetBlockEvents at offset {offset}"
        );
    }
    assert_idle(&h.state(), "the engine still answers");
}

#[test]
fn get_block_events_pages_the_stored_events() {
    let targets = ["one.example.com", "two.example.com", "three.example.com"];
    let h = harness_with("get_block_events_pages_the_stored_events", |store| {
        for (minutes, target) in (1_u64..).zip(targets) {
            let at = wall_start()
                .checked_sub(Duration::from_secs(
                    minutes.checked_mul(60).expect("in range"),
                ))
                .expect("a valid wall time");
            store
                .record_block_event(&StoredBlockEvent {
                    at,
                    kind: StoredTargetKind::Site,
                    target: target.to_owned(),
                })
                .expect("test setup: records a block event");
        }
    });

    let events = |offset: u32, limit: u32| match h.call(Request::GetBlockEvents {
        page: page(offset, limit),
    }) {
        Response::BlockEvents { items, next_offset } => (items, next_offset),
        other => panic!("GetBlockEvents should answer BlockEvents, got {other:?}"),
    };

    let (first, next) = events(0, 2);
    assert_eq!(first.len(), 2, "{first:?}");
    assert_eq!(next, Some(2));
    let (second, next) = events(2, 2);
    assert_eq!(second.len(), 1, "{second:?}");
    assert_eq!(next, None);

    let mut seen: Vec<BlockEvent> = first.into_iter().chain(second).collect();
    seen.sort_by(|a, b| a.target.cmp(&b.target));
    let mut expected: Vec<BlockEvent> = (1_u64..)
        .zip(targets)
        .map(|(minutes, target)| BlockEvent {
            at_unix_ms: (WALL_START_SECS - minutes * 60) * 1000,
            kind: TargetKind::Site,
            target: target.to_owned(),
        })
        .collect();
    expected.sort_by(|a, b| a.target.cmp(&b.target));
    assert_eq!(seen, expected, "every stored event, once");
}

#[test]
fn settings_start_at_the_default_and_can_be_updated() {
    let h = harness("settings_start_at_the_default_and_can_be_updated");
    assert_eq!(
        h.call(Request::GetSettings),
        Response::Settings {
            settings: Settings {
                default_preset: PomodoroPreset::default(),
            },
        }
    );

    let longer = preset(50, 10, 20, 2);
    h.ok(Request::UpdateSettings {
        settings: Settings {
            default_preset: longer.clone(),
        },
    });
    assert_eq!(
        h.call(Request::GetSettings),
        Response::Settings {
            settings: Settings {
                default_preset: longer.clone(),
            },
        }
    );
    assert_eq!(
        h.store_on_disk()
            .settings()
            .expect("the store reads the settings")
            .default_preset,
        longer,
        "the settings are in the store"
    );
}

// ---------------------------------------------------------------------------
// EN8 — logs never hold user data
// ---------------------------------------------------------------------------

#[test]
fn blocklists_are_never_logged() {
    let h = harness("blocklists_are_never_logged");
    let name = "Zq Secret Name 5521";
    let site = "zq-secret-site-5521.example.com";
    let secret = blocklist("zq-secret-id-5521", name, site);

    let ((), logs) = capture_logs(|| {
        h.ok(Request::UpsertBlocklist {
            blocklist: validated(secret.clone()),
        });
        assert_eq!(h.get_blocklist("zq-secret-id-5521"), Some(secret.clone()));
        let _ = h.call(Request::ListBlocklists { page: page(0, 100) });
        assert_eq!(
            h.start("zq-secret-id-5521", preset(25, 5, 15, 4), LockKind::None),
            Response::Ok
        );
        h.advance_secs(25 * 60);
        h.engine.tick();
        h.ok(Request::StopSession { text: None });
        h.ok(Request::DeleteBlocklist {
            id: "zq-secret-id-5521".to_owned(),
        });
    });
    assert!(
        !logs.contains(name),
        "a blocklist's name was logged:\n{logs}"
    );
    assert!(
        !logs.contains(site),
        "a blocklist's site was logged:\n{logs}"
    );
}

// ---------------------------------------------------------------------------
// EN9 — the engine is what the service runs
// ---------------------------------------------------------------------------

/// A pipe name no other test, or run, uses.
fn unique_pipe_name(test_name: &str) -> String {
    format!(r"\\.\pipe\keitora-t2-5-{}-{test_name}", std::process::id())
}

/// Runs `future` on a fresh runtime, failing the test if it takes too long.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(90), future)
            .await
            .expect("the test took longer than 90 s")
    })
}

/// A minimal client of the service's protocol.
struct Client {
    pipe: tokio::net::windows::named_pipe::NamedPipeClient,
    reader: FrameReader,
    next_id: u32,
}

impl Client {
    /// Connects to `name`, waiting while the pipe doesn't exist yet or is
    /// busy.
    async fn connect(name: &str) -> Self {
        use keitora_sys::named_pipe::{CLIENT_ACCESS, open_client};
        let deadline = std::time::Instant::now()
            .checked_add(WAIT)
            .expect("a deadline");
        loop {
            match open_client(name, CLIENT_ACCESS) {
                Ok(pipe) => {
                    return Self {
                        pipe,
                        reader: FrameReader::new(),
                        next_id: 0,
                    };
                }
                // ERROR_FILE_NOT_FOUND (not created yet), ERROR_PIPE_BUSY.
                Err(error)
                    if matches!(error.raw_os_error(), Some(2 | 231))
                        && std::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => panic!("couldn't connect to {name}: {error}"),
            }
        }
    }

    /// Sends `request` and waits for its response.
    async fn call(&mut self, request: Request) -> Response {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        self.next_id = self.next_id.checked_add(1).expect("an id");
        let id = format!("t-{}", self.next_id);
        let envelope = Envelope::new(
            MessageId::new(&id).expect("a valid id"),
            Message::Request(request),
        );
        self.pipe
            .write_all(&encode(&envelope).expect("encodes"))
            .await
            .expect("writes to the pipe");
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = tokio::time::timeout(WAIT, self.pipe.read(&mut buffer))
                .await
                .expect("the service answers in time")
                .expect("reads from the pipe");
            assert!(count > 0, "the service closed the connection");
            for frame in self.reader.push(buffer.get(..count).expect("in range")) {
                let frame = frame.expect("a frame within the limit");
                let envelope = decode(&frame).expect("a valid message");
                if envelope.id().as_str() == id
                    && let Message::Response(response) = envelope.message()
                {
                    return response.clone();
                }
            }
        }
    }
}

#[test]
fn the_engine_answers_requests_through_the_pipe_server() {
    let h = harness("the_engine_answers_requests_through_the_pipe_server");
    let name = unique_pipe_name("engine");
    let handler: Arc<dyn RequestHandler> = h.engine.clone();
    let server = PipeServer::start(&name, handler).expect("the pipe server starts");

    block_on(async {
        let mut client = Client::connect(&name).await;
        assert_eq!(
            client
                .call(Request::StartSession {
                    blocklist_id: WORK.to_owned(),
                    preset: preset(25, 5, 15, 4),
                    lock: LockKind::UntilTimerEnds,
                })
                .await,
            Response::Ok
        );
        assert_eq!(
            client.call(Request::GetState).await,
            Response::State {
                state: running(Phase::Focus, 1, 25 * 60, LockKind::UntilTimerEnds, WORK),
            }
        );
        assert_eq!(
            client.call(Request::StopSession { text: None }).await,
            strict_locked()
        );
    });
    server.shutdown();
    assert_eq!(
        h.state(),
        running(Phase::Focus, 1, 25 * 60, LockKind::UntilTimerEnds, WORK),
        "the pipe and the engine share one session"
    );
}

/// Kills a child process if the test ends before it exits.
#[cfg(debug_assertions)]
struct ChildGuard(std::process::Child);

#[cfg(debug_assertions)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(debug_assertions)]
#[test]
fn the_console_service_serves_the_engine_on_its_pipe_and_saves_an_idle_snapshot() {
    use std::process::{Command, Stdio};

    use keitora_service::pipe::PIPE_NAME;

    let dir =
        data_dir("the_console_service_serves_the_engine_on_its_pipe_and_saves_an_idle_snapshot");
    let sandbox = dir.parent().expect("the sandbox").to_path_buf();
    let program_data = sandbox.join("ProgramData");
    fs::create_dir_all(&program_data).expect("creates a fake ProgramData");
    let stderr = fs::File::create(sandbox.join("stderr.txt")).expect("creates a stderr file");

    let child = Command::new(env!("CARGO_BIN_EXE_keitora-service"))
        .arg("--console")
        .arg("--data-dir")
        .arg(&dir)
        .env("ProgramData", &program_data)
        .env("ALLUSERSPROFILE", &program_data)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("the service starts");
    let mut child = ChildGuard(child);

    let state = block_on(async {
        let mut client = Client::connect(PIPE_NAME).await;
        client.call(Request::GetState).await
    });
    match state {
        Response::State { state } => assert_idle(&state, "a fresh service"),
        other => panic!("GetState over the service's pipe answered {other:?}"),
    }

    // Closing standard input stops console mode cleanly (T-2.1).
    drop(child.0.stdin.take());
    let deadline = std::time::Instant::now()
        .checked_add(WAIT)
        .expect("a deadline");
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("checks the service") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the service didn't stop after its standard input closed"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let errors = fs::read_to_string(sandbox.join("stderr.txt")).unwrap_or_default();
    assert!(status.success(), "the service failed: {status}\n{errors}");

    let idle = keitora_core::engine::Engine::new().snapshot(&TestClock::new(wall_start()));
    assert_eq!(
        Store::open(&dir)
            .expect("the service's store opens")
            .load_snapshot()
            .expect("the snapshot reads"),
        Some(idle),
        "the service saved an idle snapshot on its first start"
    );
}

// ---------------------------------------------------------------------------
// A check of the harness itself
// ---------------------------------------------------------------------------

/// The log tests above prove something only if the capture sees what the
/// code under test logs on the calling thread.
#[test]
fn log_capture_sees_this_threads_events() {
    let ((), logs) = capture_logs(|| tracing::warn!("zq-capture-check"));
    assert!(logs.contains("zq-capture-check"), "{logs}");
    assert!(logs.contains("WARN"), "{logs}");
}
