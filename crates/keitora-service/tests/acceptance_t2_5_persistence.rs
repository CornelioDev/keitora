// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.5, persistence: the service engine saves a
//! snapshot of `keitora-core`'s engine on every transition, and the parts of
//! issues #48 and #49 that belong to this task.
//!
//! Requests, events, strict-mode rules and the API of
//! `keitora_service::engine` are described in `acceptance_t2_5.rs`; this
//! file uses the same API: `ServiceEngine::new(store, clock, events)`,
//! `ServiceEngine::tick`, `RequestHandler::handle` and `EventSink`.
//!
//! What the store holds is read through a second `Store` opened on the same
//! folder (`Store::load_snapshot`, which keeps returning the
//! `EngineState` that T-2.2 defined) and compared with the snapshot that
//! `keitora-core`'s own engine takes after the same steps on its own clock.
//!
//! # Decisions taken for this task (standing authorization)
//!
//! - **"Every transition"** is: a start, a pause, a resume, a skipped break,
//!   a stop, a phase change and the end of a session. Each saves a snapshot
//!   before the request is answered or, for what time does, during the tick
//!   that sees it. A tick also saves a checkpoint of the active time while a
//!   session runs: one at least 60 seconds after the last is always due
//!   (ADR-0011 says "every few seconds").
//! - **First start.** Creating the engine on a store with no snapshot saves
//!   the idle one, so that the fixed slot exists before any strict start
//!   (ADR-0013). If that save fails, the engine is created all the same: a
//!   full disk at boot must not stop the service. A store that already holds
//!   a snapshot is never overwritten by creating the engine (restoring it is
//!   T-2.7's).
//! - **#48, the first snapshot of a strict session.** If it can't be saved,
//!   `StartSession` answers `internal`, the engine stays idle and nothing is
//!   published. What an unlocked or `RandomText` start does when its
//!   snapshot fails is left to the developer.
//! - **#48, a later checkpoint that fails** is logged at `WARN` or `ERROR`
//!   level, the session goes on in memory, strict as before, and the
//!   checkpoint is tried again by later ticks; the store keeps the last
//!   snapshot saved. Logged on the thread that calls `tick`, since `tick` is
//!   synchronous.
//! - **#49, binding.** The blocklist id is stored inside the snapshot's
//!   value, the same fixed slot of at most `MAX_SNAPSHOT_BYTES` bytes that
//!   one `UPDATE` overwrites, so the id and the engine state are always
//!   written together. Only the id is stored, never the blocklist's name or
//!   targets. How the id is laid out in the value is the developer's, as
//!   long as `Store::load_snapshot` still reads the `EngineState`.
//!
//! # How failures are simulated
//!
//! - **A store that can't be written from the start** (a full disk at
//!   boot): as in T-2.2, the database is created, closed, marked read-only
//!   and opened again, so SQLite fails every write with `SQLITE_READONLY`.
//! - **Writes that start failing while the engine runs**: an open handle
//!   ignores a later read-only attribute, so instead a second connection
//!   holds SQLite's write lock (`BEGIN IMMEDIATE`). Every write of the
//!   engine's store then fails with `SQLITE_BUSY` after its busy timeout
//!   (rusqlite's default is 5 seconds), while reads still work. When the
//!   lock is released, writes work again.
//!
//! # Criteria covered
//!
//! - **PS1** — a snapshot is saved on every transition and as a periodic
//!   checkpoint:
//!   `starting_a_session_saves_its_snapshot_before_answering`,
//!   `pausing_and_resuming_save_a_snapshot`,
//!   `skipping_a_break_saves_a_snapshot`,
//!   `stopping_saves_an_idle_snapshot`,
//!   `a_tick_that_sees_a_phase_change_saves_a_snapshot`,
//!   `a_tick_saves_a_checkpoint_of_the_active_time`.
//! - **PS2** — a checkpoint is saved right when a session ends:
//!   `the_end_of_a_session_saves_an_idle_snapshot_at_once`.
//! - **PS3** — the idle snapshot is saved on the first start only, and its
//!   failure doesn't stop the engine:
//!   `a_new_engine_saves_an_idle_snapshot_on_a_fresh_store`,
//!   `creating_the_engine_never_overwrites_a_stored_session`,
//!   `the_engine_starts_even_when_its_store_cant_be_written`.
//! - **PS4** (#48) — a strict start whose first snapshot can't be saved is
//!   refused and changes nothing:
//!   `a_strict_start_whose_snapshot_cant_be_saved_is_refused`.
//! - **PS5** (#48) — a failed later checkpoint keeps the session, is logged
//!   and retried, and the store keeps the first snapshot meanwhile:
//!   `a_failed_checkpoint_keeps_the_strict_session_and_is_retried`.
//! - **PS6** (#49) — the snapshot is bound to its blocklist by id, in the
//!   same slot and write:
//!   `the_snapshot_slot_holds_the_session_blocklist_id`,
//!   `the_snapshot_slot_holds_the_id_and_not_a_copy_of_the_blocklist`,
//!   `a_new_session_binds_the_snapshot_to_its_own_blocklist`,
//!   `the_bound_snapshot_of_the_largest_session_fits_the_fixed_slot`.
//!
//! # Verified by review, not here
//!
//! - The strict start's snapshot is saved, with `Ok` from the store, before
//!   the start is answered or published (tested as far as a test can see:
//!   the store holds it when `handle` returns).
//! - ADR-0011 records the binding of the snapshot to its blocklist.

#![cfg(test)]

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keitora_core::engine::{Engine, EngineState, LockKind, PomodoroPreset};
use keitora_core::model::{Blocklist, SitePattern, Target};
use keitora_ipc::{Event, Phase, ProtocolError, Request, Response, SessionState};
use keitora_service::engine::{EventSink, ServiceEngine};
use keitora_service::pipe::RequestHandler;
use keitora_service::store::{DB_FILE_NAME, MAX_SNAPSHOT_BYTES, Store};
use rusqlite::{Connection, OpenFlags};

use self::fake::TestClock;

/// The id of the blocklist most tests start a session with.
const WORK: &str = "work";

/// The text only a snapshot with a session contains (as in T-2.2).
const SNAPSHOT_MARKER: &str = "\"first_cycle\"";

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
// Helpers: folders, the raw file and failing writes
// ---------------------------------------------------------------------------

/// A fresh, empty data folder for one test, named after it, under Cargo's
/// temporary folder for integration tests.
fn data_dir(test_name: &str) -> PathBuf {
    // Before any code under test can log (see `install_log_capture`).
    install_log_capture();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("acceptance_t2_5_persistence")
        .join(test_name);
    let dir = root.join("data");
    // A run that failed may have left the file read-only.
    let _ = clear_read_only(&dir.join(DB_FILE_NAME));
    match fs::remove_dir_all(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => panic!("couldn't clear the sandbox {}: {error}", root.display()),
    }
    fs::create_dir_all(&dir).expect("couldn't create the sandbox");
    dir
}

/// The database file in `dir`.
fn db_path(dir: &Path) -> PathBuf {
    dir.join(DB_FILE_NAME)
}

/// Clears the read-only attribute of `path`.
#[expect(
    clippy::permissions_set_readonly_false,
    reason = "Windows only: this clears the read-only attribute, it doesn't make the file world-writable"
)]
fn clear_read_only(path: &Path) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)
}

/// Keeps a file read-only until dropped.
struct ReadOnly(PathBuf);

impl ReadOnly {
    fn set(path: PathBuf) -> Self {
        let mut permissions = fs::metadata(&path)
            .expect("reads the file's attributes")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).expect("makes the file read-only");
        Self(path)
    }
}

impl Drop for ReadOnly {
    fn drop(&mut self) {
        // Never panic in `drop`: the sandbox is cleared on the next run.
        let _ = clear_read_only(&self.0);
    }
}

/// Holds SQLite's write lock on the database in `dir` until dropped, so that
/// every write of another connection fails with `SQLITE_BUSY` once its busy
/// timeout runs out. Reads still work.
struct WriteLock(Connection);

impl WriteLock {
    fn hold(dir: &Path) -> Self {
        let connection =
            Connection::open_with_flags(db_path(dir), OpenFlags::SQLITE_OPEN_READ_WRITE)
                .expect("opens the database directly");
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .expect("takes the write lock");
        Self(connection)
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = self.0.execute_batch("ROLLBACK");
    }
}

/// An SQL identifier, quoted.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Every `(table, column)` of the tables in the database.
fn columns(connection: &Connection) -> Vec<(String, String)> {
    let mut tables_query = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite%'")
        .expect("couldn't list the tables");
    let tables: Vec<String> = tables_query
        .query_map([], |row| row.get(0))
        .expect("couldn't list the tables")
        .collect::<Result<_, _>>()
        .expect("couldn't read a table name");
    let mut found = Vec::new();
    for table in tables {
        let mut columns_query = connection
            .prepare("SELECT name FROM pragma_table_info(?1)")
            .expect("couldn't list the columns");
        let names: Vec<String> = columns_query
            .query_map([table.as_str()], |row| row.get(0))
            .expect("couldn't list the columns")
            .collect::<Result<_, _>>()
            .expect("couldn't read a column name");
        for name in names {
            found.push((table.clone(), name));
        }
    }
    found
}

/// The whole stored value, padding included, that holds the snapshot of a
/// session. Fails the test unless exactly one stored value contains one.
fn snapshot_slot(dir: &Path) -> String {
    let connection = Connection::open_with_flags(db_path(dir), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("opens the database directly");
    let mut found = Vec::new();
    for (table, column) in columns(&connection) {
        let sql = format!(
            "SELECT CAST({column} AS TEXT) FROM {table} \
             WHERE instr(CAST({column} AS BLOB), CAST(?1 AS BLOB)) > 0",
            column = quoted(&column),
            table = quoted(&table),
        );
        let mut query = connection
            .prepare(&sql)
            .expect("couldn't search the database");
        let values: Vec<String> = query
            .query_map([SNAPSHOT_MARKER], |row| row.get(0))
            .expect("couldn't search the database")
            .collect::<Result<_, _>>()
            .expect("couldn't read a value");
        found.extend(values);
    }
    assert_eq!(
        found.len(),
        1,
        "expected one stored value with a session's snapshot, found {found:?}"
    );
    found.pop().expect("one value")
}

// ---------------------------------------------------------------------------
// Helpers: the engine and expected snapshots
// ---------------------------------------------------------------------------

/// The wall time every test clock starts at (2027-01-15T08:00:00Z).
fn wall_start() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_800_000_000))
        .expect("a valid wall time")
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

/// The snapshot `keitora-core`'s engine takes after `steps`, run on a fresh
/// engine and a fresh clock.
fn expected(steps: impl FnOnce(&mut Engine, &TestClock)) -> EngineState {
    let clock = TestClock::new(wall_start());
    let mut engine = Engine::new();
    steps(&mut engine, &clock);
    engine.snapshot(&clock)
}

/// The snapshot of an engine with no session.
fn idle() -> EngineState {
    Engine::new().snapshot(&TestClock::new(wall_start()))
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
    engine: ServiceEngine,
}

/// Puts the blocklist `WORK` and `extra` in a fresh store in a folder for
/// `test_name`, and closes it.
fn fresh_store(test_name: &str, extra: &[Blocklist]) -> PathBuf {
    let dir = data_dir(test_name);
    let mut store = Store::open(&dir).expect("test setup: the store opens");
    store
        .put_blocklist(&blocklist(WORK, "Work", "news.example.com"))
        .expect("test setup: stores a blocklist");
    for list in extra {
        store
            .put_blocklist(list)
            .expect("test setup: stores a blocklist");
    }
    dir
}

/// An engine on the store in `dir`, on a new clock.
fn engine_on(dir: PathBuf) -> Harness {
    let store = Store::open(&dir).expect("the store opens");
    let clock = TestClock::new(wall_start());
    let events = Arc::new(Recorder::default());
    let sink: Arc<dyn EventSink> = events.clone();
    let engine = ServiceEngine::new(store, clock.shared(), sink).expect("the engine starts");
    Harness {
        dir,
        clock,
        events,
        engine,
    }
}

/// An engine on a fresh store that holds the blocklist `WORK`.
fn harness(test_name: &str) -> Harness {
    engine_on(fresh_store(test_name, &[]))
}

/// An engine on a store that holds the blocklist `WORK` and can't be
/// written; the store becomes writable again when the guard is dropped.
fn read_only_harness(test_name: &str) -> (Harness, ReadOnly) {
    let dir = fresh_store(test_name, &[]);
    let read_only = ReadOnly::set(db_path(&dir));
    (engine_on(dir), read_only)
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

    /// Fails the test unless a session on `WORK` with `preset` and `lock`
    /// starts.
    fn start(&self, preset: PomodoroPreset, lock: LockKind) {
        self.ok(Request::StartSession {
            blocklist_id: WORK.to_owned(),
            preset,
            lock,
        });
    }

    /// The state `GetState` reports.
    fn state(&self) -> SessionState {
        match self.call(Request::GetState) {
            Response::State { state } => state,
            other => panic!("GetState should answer State, got {other:?}"),
        }
    }

    /// Lets `secs` seconds pass on the test clock.
    fn advance_secs(&self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs));
    }

    /// The snapshot the store on disk holds, read by a second store.
    fn stored(&self) -> Option<EngineState> {
        Store::open(&self.dir)
            .expect("the store opens a second time")
            .load_snapshot()
            .expect("the stored snapshot reads")
    }
}

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
// PS1 — a snapshot on every transition
// ---------------------------------------------------------------------------

#[test]
fn starting_a_session_saves_its_snapshot_before_answering() {
    let locks = [
        LockKind::None,
        LockKind::UntilTimerEnds,
        LockKind::RandomText { len: 8 },
    ];
    for (n, lock) in locks.into_iter().enumerate() {
        let h = harness(&format!("starting_a_session_saves_its_snapshot_{n}"));
        h.start(preset(25, 5, 15, 4), lock);
        assert_eq!(
            h.stored(),
            Some(expected(|engine, clock| {
                engine
                    .start(preset(25, 5, 15, 4), lock, clock)
                    .expect("starts");
            })),
            "the store holds the {lock:?} session as soon as the start is answered"
        );
    }
}

#[test]
fn pausing_and_resuming_save_a_snapshot() {
    let h = harness("pausing_and_resuming_save_a_snapshot");
    h.start(PomodoroPreset::default(), LockKind::None);
    h.advance_secs(10 * 60);

    h.ok(Request::PauseSession);
    let paused = expected(|engine, clock| {
        engine
            .start(PomodoroPreset::default(), LockKind::None, clock)
            .expect("starts");
        clock.advance(Duration::from_secs(10 * 60));
        engine.pause(clock).expect("pauses");
    });
    assert_eq!(h.stored(), Some(paused), "after the pause");

    h.advance_secs(3 * 60);
    h.ok(Request::ResumeSession);
    let resumed = expected(|engine, clock| {
        engine
            .start(PomodoroPreset::default(), LockKind::None, clock)
            .expect("starts");
        clock.advance(Duration::from_secs(10 * 60));
        engine.pause(clock).expect("pauses");
        clock.advance(Duration::from_secs(3 * 60));
        engine.resume(clock).expect("resumes");
    });
    assert_eq!(h.stored(), Some(resumed), "after the resume");
}

#[test]
fn skipping_a_break_saves_a_snapshot() {
    let h = harness("skipping_a_break_saves_a_snapshot");
    h.start(PomodoroPreset::default(), LockKind::None);
    h.advance_secs(26 * 60);
    h.ok(Request::SkipBreak);
    assert_eq!(
        h.stored(),
        Some(expected(|engine, clock| {
            engine
                .start(PomodoroPreset::default(), LockKind::None, clock)
                .expect("starts");
            clock.advance(Duration::from_secs(26 * 60));
            engine.skip_break(clock).expect("skips the break");
        }))
    );
}

#[test]
fn stopping_saves_an_idle_snapshot() {
    let h = harness("stopping_saves_an_idle_snapshot");
    h.start(PomodoroPreset::default(), LockKind::None);
    h.advance_secs(60);
    h.engine.tick();
    assert_ne!(h.stored(), Some(idle()), "test setup: a session is stored");

    h.ok(Request::StopSession { text: None });
    assert_eq!(h.stored(), Some(idle()));
}

#[test]
fn a_tick_that_sees_a_phase_change_saves_a_snapshot() {
    let h = harness("a_tick_that_sees_a_phase_change_saves_a_snapshot");
    h.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
    h.advance_secs(25 * 60);
    h.engine.tick();
    assert_eq!(
        h.stored(),
        Some(expected(|engine, clock| {
            engine
                .start(PomodoroPreset::default(), LockKind::UntilTimerEnds, clock)
                .expect("starts");
            clock.advance(Duration::from_secs(25 * 60));
        })),
        "the store holds the session at the start of its short break"
    );
}

#[test]
fn a_tick_saves_a_checkpoint_of_the_active_time() {
    let h = harness("a_tick_saves_a_checkpoint_of_the_active_time");
    h.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
    for minutes in 1..=3_u64 {
        h.advance_secs(60);
        h.engine.tick();
        assert_eq!(
            h.stored(),
            Some(expected(|engine, clock| {
                engine
                    .start(PomodoroPreset::default(), LockKind::UntilTimerEnds, clock)
                    .expect("starts");
                clock.advance(Duration::from_secs(minutes * 60));
            })),
            "the checkpoint after {minutes} minutes"
        );
    }
}

// ---------------------------------------------------------------------------
// PS2 — a checkpoint right when a session ends
// ---------------------------------------------------------------------------

#[test]
fn the_end_of_a_session_saves_an_idle_snapshot_at_once() {
    let h = harness("the_end_of_a_session_saves_an_idle_snapshot_at_once");
    h.start(preset(1, 1, 1, 1), LockKind::UntilTimerEnds);
    h.advance_secs(60);
    h.engine.tick();
    assert_ne!(
        h.stored(),
        Some(idle()),
        "test setup: the long break is stored"
    );

    h.advance_secs(60);
    h.engine.tick();
    assert_eq!(
        h.stored(),
        Some(idle()),
        "the tick that sees the end saves the idle engine"
    );
}

// ---------------------------------------------------------------------------
// PS3 — the idle snapshot on the first start
// ---------------------------------------------------------------------------

#[test]
fn a_new_engine_saves_an_idle_snapshot_on_a_fresh_store() {
    let h = harness("a_new_engine_saves_an_idle_snapshot_on_a_fresh_store");
    assert_eq!(h.stored(), Some(idle()));
    assert_eq!(h.state().phase, Phase::Idle);
}

#[test]
fn creating_the_engine_never_overwrites_a_stored_session() {
    let dir = fresh_store("creating_the_engine_never_overwrites_a_stored_session", &[]);
    let first = engine_on(dir.clone());
    first.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
    first.advance_secs(10 * 60);
    first.engine.tick();
    let saved = first.stored();
    assert_ne!(
        saved,
        Some(idle()),
        "test setup: a strict session is stored"
    );
    drop(first);

    // The service starts again, on a new clock.
    let second = engine_on(dir);
    assert_eq!(
        second.stored(),
        saved,
        "creating the engine kept the stored strict session"
    );
}

#[test]
fn the_engine_starts_even_when_its_store_cant_be_written() {
    let (h, _read_only) =
        read_only_harness("the_engine_starts_even_when_its_store_cant_be_written");
    assert_eq!(h.state().phase, Phase::Idle, "the engine answers");
    assert_eq!(h.stored(), None, "nothing could be saved");
}

// ---------------------------------------------------------------------------
// PS4 — #48: a strict start needs its first snapshot
// ---------------------------------------------------------------------------

#[test]
fn a_strict_start_whose_snapshot_cant_be_saved_is_refused() {
    let (h, _read_only) =
        read_only_harness("a_strict_start_whose_snapshot_cant_be_saved_is_refused");
    h.events.take();

    for preset in [PomodoroPreset::default(), preset(1, 1, 1, 1)] {
        assert_eq!(
            h.call(Request::StartSession {
                blocklist_id: WORK.to_owned(),
                preset,
                lock: LockKind::UntilTimerEnds,
            }),
            Response::Error {
                error: ProtocolError::Internal,
            },
            "a strict start that can't be saved"
        );
        let state = h.state();
        assert_eq!(state.phase, Phase::Idle, "the engine stays idle: {state:?}");
        assert_eq!(state.lock, None, "{state:?}");
    }
    assert_eq!(h.events.take(), Vec::new(), "nothing is published");
    assert_eq!(
        h.call(Request::StopSession { text: None }),
        Response::Error {
            error: ProtocolError::InvalidState,
        },
        "there is no session to stop"
    );
    assert_eq!(h.stored(), None, "nothing was saved");
}

// ---------------------------------------------------------------------------
// PS5 — #48: a failed checkpoint never drops the session
// ---------------------------------------------------------------------------

#[test]
fn a_failed_checkpoint_keeps_the_strict_session_and_is_retried() {
    let h = harness("a_failed_checkpoint_keeps_the_strict_session_and_is_retried");
    h.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
    let first = expected(|engine, clock| {
        engine
            .start(PomodoroPreset::default(), LockKind::UntilTimerEnds, clock)
            .expect("starts");
    });
    assert_eq!(
        h.stored(),
        Some(first.clone()),
        "test setup: the start is stored"
    );

    // Writes start failing.
    let lock = WriteLock::hold(&h.dir);
    h.advance_secs(60);
    let ((), logs) = capture_logs(|| h.engine.tick());
    assert!(
        logs.contains("WARN") || logs.contains("ERROR"),
        "a failed checkpoint is logged as a warning or an error:\n{logs}"
    );

    // The session goes on in memory, still strict.
    let state = h.state();
    assert_eq!(state.phase, Phase::Focus, "{state:?}");
    assert_eq!(state.remaining_ms, 24 * 60 * 1000, "{state:?}");
    assert_eq!(state.lock, Some(LockKind::UntilTimerEnds), "{state:?}");
    assert_eq!(state.blocklist_id.as_deref(), Some(WORK), "{state:?}");
    assert_eq!(
        h.call(Request::StopSession { text: None }),
        Response::Error {
            error: ProtocolError::StrictSessionLocked {
                reason: keitora_ipc::LockReason::UntilTimerEnds,
            },
        },
        "still strict"
    );

    // The store still holds the first snapshot.
    drop(lock);
    assert_eq!(
        h.stored(),
        Some(first),
        "the failed checkpoint left the first snapshot whole"
    );

    // Once writes work again, a later tick saves the session.
    h.advance_secs(60);
    h.engine.tick();
    assert_eq!(
        h.stored(),
        Some(expected(|engine, clock| {
            engine
                .start(PomodoroPreset::default(), LockKind::UntilTimerEnds, clock)
                .expect("starts");
            clock.advance(Duration::from_secs(2 * 60));
        })),
        "the checkpoint is saved once writes work again"
    );
}

// ---------------------------------------------------------------------------
// PS6 — #49: the snapshot is bound to its blocklist
// ---------------------------------------------------------------------------

#[test]
fn the_snapshot_slot_holds_the_session_blocklist_id() {
    let h = harness("the_snapshot_slot_holds_the_session_blocklist_id");
    h.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
    let slot = snapshot_slot(&h.dir);
    assert!(
        slot.contains("\"work\""),
        "the snapshot's slot names the blocklist `work`: {slot:?}"
    );
    assert!(
        slot.len() <= MAX_SNAPSHOT_BYTES,
        "the slot is {} bytes long, more than {MAX_SNAPSHOT_BYTES}",
        slot.len()
    );
}

#[test]
fn the_snapshot_slot_holds_the_id_and_not_a_copy_of_the_blocklist() {
    let name = "Zq Bound Name 4949";
    let site = "zq-bound-site-4949.example.com";
    let dir = fresh_store(
        "the_snapshot_slot_holds_the_id_and_not_a_copy_of_the_blocklist",
        &[blocklist("deep-work-4949", name, site)],
    );
    let h = engine_on(dir);
    h.ok(Request::StartSession {
        blocklist_id: "deep-work-4949".to_owned(),
        preset: PomodoroPreset::default(),
        lock: LockKind::UntilTimerEnds,
    });

    let slot = snapshot_slot(&h.dir);
    assert!(
        slot.contains("\"deep-work-4949\""),
        "the slot names the blocklist `deep-work-4949`: {slot:?}"
    );
    assert!(!slot.contains(name), "the slot copies the name: {slot:?}");
    assert!(!slot.contains(site), "the slot copies a target: {slot:?}");
}

#[test]
fn a_new_session_binds_the_snapshot_to_its_own_blocklist() {
    let dir = fresh_store(
        "a_new_session_binds_the_snapshot_to_its_own_blocklist",
        &[
            blocklist("first-4949", "First", "first.example.com"),
            blocklist("second-4949", "Second", "second.example.com"),
        ],
    );
    let h = engine_on(dir);
    h.ok(Request::StartSession {
        blocklist_id: "first-4949".to_owned(),
        preset: PomodoroPreset::default(),
        lock: LockKind::None,
    });
    let slot = snapshot_slot(&h.dir);
    assert!(
        slot.contains("\"first-4949\""),
        "the slot names the blocklist `first-4949`: {slot:?}"
    );

    h.ok(Request::StopSession { text: None });
    h.ok(Request::StartSession {
        blocklist_id: "second-4949".to_owned(),
        preset: PomodoroPreset::default(),
        lock: LockKind::UntilTimerEnds,
    });
    let slot = snapshot_slot(&h.dir);
    assert!(
        slot.contains("\"second-4949\""),
        "the slot names the blocklist `second-4949`: {slot:?}"
    );
    assert!(
        !slot.contains("first-4949"),
        "the slot still names the first blocklist: {slot:?}"
    );
}

#[test]
fn the_bound_snapshot_of_the_largest_session_fits_the_fixed_slot() {
    // The longest id a blocklist may have, and the longest snapshot the
    // engine writes: the longest preset, the longest random text and an
    // active time just before the end, with every digit of the nanoseconds.
    let id = "a".repeat(keitora_core::model::MAX_ID_LEN);
    let dir = fresh_store(
        "the_bound_snapshot_of_the_largest_session_fits_the_fixed_slot",
        &[blocklist(&id, "Longest", "longest.example.com")],
    );
    let h = engine_on(dir);
    let longest = preset(120, 30, 60, 8);
    let lock = LockKind::RandomText { len: 256 };
    // 8 × 120 + 7 × 30 + 60 = 1230 minutes in all.
    let active = Duration::new(1_230 * 60 - 1, 999_999_999);
    h.ok(Request::StartSession {
        blocklist_id: id.clone(),
        preset: longest.clone(),
        lock,
    });
    h.clock.advance(active);
    h.engine.tick();

    let slot = snapshot_slot(&h.dir);
    assert!(
        slot.contains(&format!("\"{id}\"")),
        "the slot names the blocklist with the longest id: {slot:?}"
    );
    assert!(
        slot.len() <= MAX_SNAPSHOT_BYTES,
        "the slot is {} bytes long, more than {MAX_SNAPSHOT_BYTES}",
        slot.len()
    );
    assert_eq!(
        h.stored(),
        Some(expected(|engine, clock| {
            engine.start(longest, lock, clock).expect("starts");
            clock.advance(active);
        })),
        "the store still reads the engine state"
    );
}
