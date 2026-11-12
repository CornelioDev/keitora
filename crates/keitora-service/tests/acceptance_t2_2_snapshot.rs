// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.2: the store keeps the engine snapshot
//! (`keitora_core::engine::EngineState`, ADR-0011), including what issue #48
//! requires so that a full disk can't erase a strict session.
//!
//! Blocklists, settings, history, block events and migrations are tested in
//! `acceptance_t2_2.rs`, whose header describes the rest of the
//! `keitora_service::store` API, the storage form the tamper helpers rely
//! on, and the variants of `StoreError`.
//!
//! # Expected public API
//!
//! In `keitora_service::store`:
//!
//! - `MAX_SNAPSHOT_BYTES: usize`, the largest stored snapshot `load_snapshot`
//!   reads, in bytes. At least the largest snapshot the engine can write
//!   (tested), and at most 64 KiB (tested).
//! - `Store::save_snapshot(&mut self, state: &EngineState) -> Result<(),
//!   StoreError>` replaces the stored snapshot. `Ok` means it is durably on
//!   disk (flushed, as with `PRAGMA synchronous = FULL`), so T-2.5 can refuse
//!   to start a strict session when this fails (#48). On error, the snapshot
//!   stored before is still the one stored, whole: the write is atomic.
//! - `Store::load_snapshot(&self) -> Result<Option<EngineState>,
//!   StoreError>`: `None` if no snapshot was ever saved. A stored value
//!   longer than `MAX_SNAPSHOT_BYTES` is `StoreError::SnapshotTooLarge`,
//!   checked on its length before it is parsed; one that isn't valid
//!   `EngineState` JSON is `StoreError::Corrupt`. Restoring and checking the
//!   session (`Engine::restore`) is T-2.7's.
//!
//! The snapshot is stored in `keitora.db` as the JSON text of `EngineState`
//! (ADR-0011), alone in its value, optionally followed by ASCII whitespace
//! (so that a fixed-size slot can be padded). The tests find it by the text
//! `"first_cycle"`, which only a snapshot with a session contains, and
//! replace the whole value.
//!
//! # Decisions taken for this task (standing authorization)
//!
//! - **A store that can't write still opens and reads.** A full disk at boot
//!   must not stop the service from restoring a strict session, so opening
//!   an up-to-date database whose file can't be written succeeds, every read
//!   works, and every write returns `StoreError::Database`.
//! - **How "writes fail" is simulated:** the tests close the store, set the
//!   read-only attribute on `keitora.db`, and open it again (an open file
//!   handle ignores a later attribute change, so the store has to be
//!   reopened). SQLite then fails every write with `SQLITE_READONLY`. This
//!   is a real operating-system failure, but not `SQLITE_FULL`, and it
//!   happens before anything is written, so it shows that a failed write
//!   reports an error and loses nothing; it doesn't show what a write torn
//!   halfway by a full disk or a power cut leaves behind. That atomicity
//!   rests on SQLite's journal and on `synchronous`, checked by review.
//! - **How "no new disk space" is checked:** after the first strict
//!   snapshot, the tests create a folder with the name of each SQLite side
//!   file that doesn't exist yet (`keitora.db-journal`, `-wal`, `-shm`), so
//!   SQLite can't create it, and then save many checkpoints, whose JSON gets
//!   longer as the active time grows, and the snapshot of the session's end.
//!   Every save must succeed; afterwards no file in the folder may have
//!   appeared, disappeared or grown, and no SQLite side file may have been
//!   empty when the checkpoints began (an empty journal or WAL needs new
//!   space on the next write). On a real full disk a new journal or a longer
//!   file needs new clusters, so a design that passes doesn't depend on
//!   them. Limits: a file that grows during a save and shrinks back before
//!   it returns (a journal truncated at commit to a non-zero size limit) is
//!   not seen; `PRAGMA max_page_count` was not used because it only bounds
//!   the main file, not the journal or WAL, and because the store's own
//!   connection can't be reached from a test. This covers the snapshot
//!   only: other writes (history, block events) may need new space.
//! - Issue #48's other halves are T-2.5's: refusing the start when the first
//!   strict snapshot fails, and keeping a session in memory, logging and
//!   retrying when a checkpoint fails.
//!
//! # Criteria covered
//!
//! - **SN1** — the store saves and loads the engine snapshot, identical,
//!   across reopening; a new store has none:
//!   `a_new_store_has_no_snapshot`,
//!   `a_saved_snapshot_reads_back_identical_after_reopening`,
//!   `saving_a_snapshot_replaces_the_previous_one`,
//!   `the_largest_snapshot_the_engine_writes_fits_under_the_cap`.
//! - **SN2** — the snapshot's size is capped before parsing, at most 64 KiB:
//!   `a_stored_snapshot_over_the_cap_is_refused_even_if_it_is_valid_json`,
//!   `a_stored_snapshot_over_the_cap_is_refused_before_it_is_parsed`,
//!   `a_stored_snapshot_of_exactly_the_cap_is_read`.
//! - **SN3** — a stored snapshot that isn't valid `EngineState` JSON is a
//!   typed error: `a_malformed_stored_snapshot_is_reported_as_corrupt`.
//! - **SN4** (#48) — a strict session's first snapshot that can't be written
//!   is an error, and nothing of it is kept:
//!   `a_strict_snapshot_that_cant_be_written_is_an_error_and_keeps_the_previous_one`,
//!   `a_strict_snapshot_that_cant_be_written_leaves_no_snapshot_in_a_new_store`.
//! - **SN5** (#48) — with writes failing after the first strict snapshot, a
//!   checkpoint is an error, and the store still opens and yields the first
//!   snapshot: `a_failed_checkpoint_keeps_the_first_strict_snapshot`.
//! - **SN6** (#48) — later checkpoints of a strict session need no new disk
//!   space, also after the service restarts:
//!   `checkpoints_of_a_strict_session_need_no_new_disk_space`,
//!   `checkpoints_after_reopening_a_strict_session_need_no_new_disk_space`.
//!
//! # Verified by review, not here
//!
//! - Durability: the connection uses `synchronous = FULL` (or `EXTRA`), so
//!   `Ok` from `save_snapshot` means flushed; a power cut can't be simulated
//!   in a test.
//! - The size check reads the value's length (`length()` in SQL, or the
//!   blob's size) before reading or parsing the value itself.
//! - No `MonotonicInstant` or wall time is stored with the snapshot.

#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use keitora_core::engine::{Engine, EngineState, LockKind, PomodoroPreset};
use keitora_service::clock::SystemClock;
use keitora_service::store::{DB_FILE_NAME, MAX_SNAPSHOT_BYTES, Store, StoreError};
use rusqlite::{Connection, OpenFlags};

/// The text only a snapshot with a session contains.
const SNAPSHOT_MARKER: &str = "\"first_cycle\"";

/// The side files SQLite may keep next to the database.
const SIDE_FILES: [&str; 3] = ["keitora.db-journal", "keitora.db-wal", "keitora.db-shm"];

// ---------------------------------------------------------------------------
// Helpers: folders, the raw file and its read-only attribute
// ---------------------------------------------------------------------------

/// A fresh, empty data folder for one test, named after it, under Cargo's
/// temporary folder for integration tests.
fn data_dir(test_name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("acceptance_t2_2_snapshot")
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

/// Opens the store in `dir`, failing the test if it can't.
fn open(dir: &Path) -> Store {
    Store::open(dir).unwrap_or_else(|error| panic!("couldn't open the store: {error:?}"))
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

/// A direct connection to the database file in `dir`. Use it only while no
/// `Store` is open on the file.
fn raw_connection(dir: &Path) -> Connection {
    Connection::open_with_flags(db_path(dir), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .expect("couldn't open the database file directly")
}

/// An SQL identifier, quoted.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Every `(table, column)` of the tables the store created.
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

/// The stored snapshot's JSON, without trailing whitespace. Fails the test
/// unless exactly one stored value contains a snapshot with a session.
fn stored_snapshot_json(dir: &Path) -> String {
    let connection = raw_connection(dir);
    let mut found = Vec::new();
    for (table, column) in columns(&connection) {
        let table = quoted(&table);
        let column = quoted(&column);
        let sql = format!(
            "SELECT CAST({column} AS TEXT) FROM {table} \
             WHERE instr(CAST({column} AS BLOB), CAST(?1 AS BLOB)) > 0"
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
        "test setup: expected one stored snapshot with a session, found {found:?}"
    );
    let json = found.pop().expect("one value");
    let json = json.trim_end().to_owned();
    assert!(
        json.starts_with('{'),
        "the snapshot must be stored as its JSON text: {json:?}"
    );
    json
}

/// Replaces every stored value that contains a snapshot with a session by
/// `content`, keeping each value's type (text or blob).
fn overwrite_stored_snapshot(dir: &Path, content: &str) {
    let connection = raw_connection(dir);
    connection
        .execute_batch("PRAGMA ignore_check_constraints = ON;")
        .expect("couldn't turn off CHECK constraints");
    let mut changed: usize = 0;
    for (table, column) in columns(&connection) {
        let table = quoted(&table);
        let column = quoted(&column);
        let sql = format!(
            "UPDATE {table} SET {column} = CASE typeof({column}) \
               WHEN 'blob' THEN CAST(?1 AS BLOB) ELSE ?1 END \
             WHERE instr(CAST({column} AS BLOB), CAST(?2 AS BLOB)) > 0"
        );
        let rows = connection
            .execute(&sql, [content, SNAPSHOT_MARKER])
            .expect("couldn't tamper with the database");
        changed = changed.checked_add(rows).expect("too many rows");
    }
    assert!(
        changed > 0,
        "test setup: no stored snapshot with a session found"
    );
}

/// Every regular file in `dir`, by name, with its size in bytes.
fn files_in(dir: &Path) -> BTreeMap<String, u64> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(dir).expect("lists the data folder") {
        let entry = entry.expect("reads a folder entry");
        let metadata = entry.metadata().expect("reads an entry's metadata");
        if metadata.is_file() {
            let name = entry.file_name().into_string().expect("a UTF-8 name");
            files.insert(name, metadata.len());
        }
    }
    files
}

/// Puts a folder where each SQLite side file that doesn't exist yet would
/// go, so that SQLite can't create it, as on a full disk.
fn block_new_side_files(dir: &Path) {
    for name in SIDE_FILES {
        let path = dir.join(name);
        if !path.exists() {
            fs::create_dir(&path).expect("creates a blocking folder");
        }
    }
}

/// Removes the folders `block_new_side_files` created.
fn unblock_side_files(dir: &Path) {
    for name in SIDE_FILES {
        let path = dir.join(name);
        if path.is_dir() {
            fs::remove_dir(&path).expect("removes a blocking folder");
        }
    }
}

/// Fails the test if an SQLite side file is empty: its next write would need
/// new disk space.
fn assert_side_files_reserve_space(files: &BTreeMap<String, u64>) {
    for (name, len) in files {
        if name.starts_with("keitora.db-") {
            assert!(
                *len > 0,
                "{name} is empty, so the next checkpoint needs new disk space for it: {files:?}"
            );
        }
    }
}

/// Fails the test if a file in `dir` appeared, disappeared or grew since
/// `before`.
fn assert_no_new_space(dir: &Path, before: &BTreeMap<String, u64>, when: &str) {
    let now = files_in(dir);
    assert_eq!(
        now.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "{when}: files appeared or disappeared (before {before:?}, now {now:?})"
    );
    for (name, len) in &now {
        assert!(
            len <= &before[name],
            "{when}: {name} grew from {} to {len} bytes, which needs new disk space",
            before[name]
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers: snapshots
// ---------------------------------------------------------------------------

/// The snapshot of a session started with `preset` and `lock`, after
/// `active` time, paused or not.
#[expect(
    clippy::disallowed_types,
    reason = "a test needs a clock it moves by hand; production code never uses FakeClock"
)]
fn snapshot_after(
    preset: PomodoroPreset,
    lock: LockKind,
    active: Duration,
    paused: bool,
) -> EngineState {
    let clock = keitora_core::clock::FakeClock::new(UNIX_EPOCH);
    let mut engine = Engine::new();
    engine
        .start(preset, lock, &clock)
        .expect("test setup: the session starts");
    clock
        .advance(active)
        .expect("test setup: the clock advances");
    if paused {
        engine
            .pause(&clock)
            .expect("test setup: the session pauses");
    }
    let state = engine.snapshot(&clock);
    assert_ne!(state, idle(), "test setup: the session had already ended");
    state
}

/// A strict session with the default preset (25/5/15, 4 cycles: 130 min),
/// `active` into it.
fn strict(active: Duration) -> EngineState {
    snapshot_after(
        PomodoroPreset::default(),
        LockKind::UntilTimerEnds,
        active,
        false,
    )
}

/// The snapshot of an engine with no session.
fn idle() -> EngineState {
    Engine::new().snapshot(&SystemClock::new())
}

/// The snapshot with the longest JSON the engine can write: the longest
/// preset, a random-text lock of the longest length and an active time just
/// before the session's end, with every digit of the nanoseconds.
fn largest_snapshot() -> EngineState {
    // 8 × 120 + 7 × 30 + 60 = 1230 minutes in all.
    snapshot_after(
        PomodoroPreset::new(120, 30, 60, 8).expect("valid"),
        LockKind::RandomText { len: 256 },
        Duration::new(1_230 * 60 - 1, 999_999_999),
        false,
    )
}

// ---------------------------------------------------------------------------
// SN1 — save and load
// ---------------------------------------------------------------------------

#[test]
fn a_new_store_has_no_snapshot() {
    let dir = data_dir("no_snapshot");
    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), None);
    drop(store);
    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), None);
}

#[test]
fn a_saved_snapshot_reads_back_identical_after_reopening() {
    let states = [
        idle(),
        strict(Duration::ZERO),
        strict(Duration::new(25 * 60 + 3, 141_592_653)),
        snapshot_after(
            PomodoroPreset::new(50, 10, 30, 2).expect("valid"),
            LockKind::None,
            Duration::from_secs(7 * 60),
            true,
        ),
        snapshot_after(
            PomodoroPreset::new(1, 1, 1, 1).expect("valid"),
            LockKind::RandomText { len: 12 },
            Duration::from_millis(59_999),
            false,
        ),
    ];
    for (index, state) in states.into_iter().enumerate() {
        let dir = data_dir(&format!("round_trip_{index}"));
        let mut store = open(&dir);
        store.save_snapshot(&state).expect("saves");
        assert_eq!(store.load_snapshot().expect("reads"), Some(state.clone()));
        drop(store);

        let store = open(&dir);
        assert_eq!(
            store.load_snapshot().expect("reads"),
            Some(state),
            "snapshot {index} changed across reopening"
        );
    }
}

#[test]
fn saving_a_snapshot_replaces_the_previous_one() {
    let dir = data_dir("replace");
    let first = strict(Duration::from_secs(60));
    let second = strict(Duration::from_secs(120));
    let mut store = open(&dir);

    store.save_snapshot(&first).expect("saves");
    store.save_snapshot(&second).expect("saves");
    assert_eq!(store.load_snapshot().expect("reads"), Some(second.clone()));
    store.save_snapshot(&idle()).expect("saves");
    assert_eq!(store.load_snapshot().expect("reads"), Some(idle()));
    store.save_snapshot(&second).expect("saves");
    drop(store);

    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), Some(second));
}

#[test]
fn the_largest_snapshot_the_engine_writes_fits_under_the_cap() {
    let dir = data_dir("largest");
    let largest = largest_snapshot();
    let mut store = open(&dir);
    store.save_snapshot(&largest).expect("saves");
    drop(store);

    assert!(stored_snapshot_json(&dir).len() <= MAX_SNAPSHOT_BYTES);
    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), Some(largest));
}

// ---------------------------------------------------------------------------
// SN2, SN3 — the size cap and malformed snapshots
// ---------------------------------------------------------------------------

/// Saves a strict snapshot in a fresh store in `dir`, closes it, and
/// replaces the stored value by `content`.
fn store_raw_snapshot(dir: &Path, content: &str) {
    let mut store = open(dir);
    store
        .save_snapshot(&strict(Duration::from_secs(60)))
        .expect("saves");
    drop(store);
    overwrite_stored_snapshot(dir, content);
}

/// `{"session":null,"pad":"xxx…"}`, exactly `len` bytes: valid
/// `EngineState` JSON for an idle engine, with an unknown field.
fn idle_json_of_len(len: usize) -> String {
    let frame = r#"{"session":null,"pad":""}"#;
    let pad = len.checked_sub(frame.len()).expect("test setup: too short");
    format!(r#"{{"session":null,"pad":"{}"}}"#, "x".repeat(pad))
}

#[test]
fn a_stored_snapshot_over_the_cap_is_refused_even_if_it_is_valid_json() {
    // The real snapshot, with an unknown field in front that a parser would
    // skip.
    let dir = data_dir("over_cap_valid");
    let mut store = open(&dir);
    store
        .save_snapshot(&strict(Duration::from_secs(60)))
        .expect("saves");
    drop(store);
    let json = stored_snapshot_json(&dir);
    let rest = json.get(1..).expect("starts with `{`");
    let frame = r#"{"pad":"","#.len() + rest.len();
    let oversized = format!(
        r#"{{"pad":"{}",{rest}"#,
        "x".repeat(MAX_SNAPSHOT_BYTES + 1 - frame)
    );
    assert_eq!(oversized.len(), MAX_SNAPSHOT_BYTES + 1);
    overwrite_stored_snapshot(&dir, &oversized);

    let store = open(&dir);
    let loaded = store.load_snapshot();
    assert!(
        matches!(loaded, Err(StoreError::SnapshotTooLarge { .. })),
        "expected StoreError::SnapshotTooLarge, got {loaded:?}"
    );
}

#[test]
fn a_stored_snapshot_over_the_cap_is_refused_before_it_is_parsed() {
    // Deeply nested JSON: a parser would fail on it (or recurse deeply), so
    // only a check made before parsing reports its size.
    let nested = "[".repeat(MAX_SNAPSHOT_BYTES + 1);
    // 64 KiB and one byte, valid: the cap is at most 64 KiB.
    let over_64_kib = idle_json_of_len(64 * 1024 + 1);

    for (name, content) in [("nested", nested), ("over_64_kib", over_64_kib)] {
        let dir = data_dir(&format!("over_cap_{name}"));
        store_raw_snapshot(&dir, &content);

        let store = open(&dir);
        let loaded = store.load_snapshot();
        assert!(
            matches!(loaded, Err(StoreError::SnapshotTooLarge { .. })),
            "{name}: expected StoreError::SnapshotTooLarge, got {loaded:?}"
        );
    }
}

#[test]
fn a_stored_snapshot_of_exactly_the_cap_is_read() {
    let dir = data_dir("at_cap");
    let content = idle_json_of_len(MAX_SNAPSHOT_BYTES);
    assert_eq!(content.len(), MAX_SNAPSHOT_BYTES);
    store_raw_snapshot(&dir, &content);

    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), Some(idle()));
}

#[test]
fn a_malformed_stored_snapshot_is_reported_as_corrupt() {
    let cases = [
        ("empty", ""),
        ("cut_short", r#"{"session":{"preset":{"focus_min":25,"#),
        ("not_an_object", "42"),
        (
            "preset_out_of_range",
            r#"{"session":{"preset":{"focus_min":0,"short_break_min":5,"long_break_min":15,"cycles":4},"lock":{"kind":"until_timer_ends"},"first_cycle":1,"active":{"secs":0,"nanos":0},"paused":false}}"#,
        ),
        (
            "missing_lock",
            r#"{"session":{"preset":{"focus_min":25,"short_break_min":5,"long_break_min":15,"cycles":4},"first_cycle":1,"active":{"secs":0,"nanos":0},"paused":false}}"#,
        ),
    ];
    for (name, content) in cases {
        let dir = data_dir(&format!("malformed_{name}"));
        store_raw_snapshot(&dir, content);

        let store = open(&dir);
        let loaded = store.load_snapshot();
        assert!(
            matches!(loaded, Err(StoreError::Corrupt { .. })),
            "{name}: expected StoreError::Corrupt, got {loaded:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// SN4, SN5 — writes that fail (#48)
// ---------------------------------------------------------------------------

#[test]
fn a_strict_snapshot_that_cant_be_written_is_an_error_and_keeps_the_previous_one() {
    let dir = data_dir("strict_start_fails");
    let previous = snapshot_after(
        PomodoroPreset::default(),
        LockKind::None,
        Duration::from_secs(90),
        true,
    );
    let mut store = open(&dir);
    store.save_snapshot(&previous).expect("saves");
    drop(store);

    {
        let _read_only = ReadOnly::set(db_path(&dir));
        let mut store = open(&dir);
        assert_eq!(
            store.load_snapshot().expect("reads"),
            Some(previous.clone())
        );

        let saved = store.save_snapshot(&strict(Duration::ZERO));
        assert!(
            matches!(saved, Err(StoreError::Database { .. })),
            "a strict snapshot that wasn't written must be reported, got {saved:?}"
        );
        assert_eq!(
            store.load_snapshot().expect("reads"),
            Some(previous.clone()),
            "the failed write changed the stored snapshot"
        );
    }

    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), Some(previous));
}

#[test]
fn a_strict_snapshot_that_cant_be_written_leaves_no_snapshot_in_a_new_store() {
    let dir = data_dir("strict_start_fails_new_store");
    drop(open(&dir));

    {
        let _read_only = ReadOnly::set(db_path(&dir));
        let mut store = open(&dir);
        let saved = store.save_snapshot(&strict(Duration::ZERO));
        assert!(
            matches!(saved, Err(StoreError::Database { .. })),
            "expected StoreError::Database, got {saved:?}"
        );
        assert_eq!(store.load_snapshot().expect("reads"), None);
    }

    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), None);
}

#[test]
fn a_failed_checkpoint_keeps_the_first_strict_snapshot() {
    let dir = data_dir("checkpoint_fails");
    let first = strict(Duration::ZERO);
    let mut store = open(&dir);
    store
        .save_snapshot(&first)
        .expect("the first strict snapshot is saved");
    drop(store);

    {
        let _read_only = ReadOnly::set(db_path(&dir));
        // The disk is full when the service starts again: it still opens
        // the store and restores the session.
        let mut store = open(&dir);
        assert_eq!(store.load_snapshot().expect("reads"), Some(first.clone()));

        for minute in 1..=3 {
            let saved = store.save_snapshot(&strict(Duration::from_secs(minute * 60)));
            assert!(
                matches!(saved, Err(StoreError::Database { .. })),
                "a checkpoint that wasn't written must be reported, got {saved:?}"
            );
        }
        let ended = store.save_snapshot(&idle());
        assert!(
            matches!(ended, Err(StoreError::Database { .. })),
            "expected StoreError::Database, got {ended:?}"
        );
        assert_eq!(store.load_snapshot().expect("reads"), Some(first.clone()));
    }

    let store = open(&dir);
    assert_eq!(
        store.load_snapshot().expect("reads"),
        Some(first),
        "the strict session was lost"
    );
}

// ---------------------------------------------------------------------------
// SN6 — checkpoints need no new disk space (#48)
// ---------------------------------------------------------------------------

/// Saves checkpoints of the default strict session, each with more active
/// time and longer JSON than the first, then the snapshot of its end, and
/// checks that none needed new disk space.
fn checkpoint_without_new_space(dir: &Path, store: &mut Store) {
    block_new_side_files(dir);
    let before = files_in(dir);
    assert_side_files_reserve_space(&before);

    let mut checkpoints: Vec<EngineState> = (1..=20)
        .map(|minute: u64| strict(Duration::from_secs(minute.checked_mul(61).expect("small"))))
        .collect();
    // 130 minutes in all: the last checkpoint is just before the end, with
    // the longest active time this session can have.
    checkpoints.push(strict(Duration::new(130 * 60 - 1, 999_999_999)));

    for (index, checkpoint) in checkpoints.iter().enumerate() {
        store
            .save_snapshot(checkpoint)
            .unwrap_or_else(|error| panic!("checkpoint {index} needed new disk space: {error:?}"));
        assert_no_new_space(dir, &before, &format!("checkpoint {index}"));
    }
    store
        .save_snapshot(&idle())
        .unwrap_or_else(|error| panic!("the session's end needed new disk space: {error:?}"));
    assert_no_new_space(dir, &before, "the session's end");
    assert_eq!(store.load_snapshot().expect("reads"), Some(idle()));
}

#[test]
fn checkpoints_of_a_strict_session_need_no_new_disk_space() {
    let dir = data_dir("in_place");
    let mut store = open(&dir);
    store
        .save_snapshot(&strict(Duration::ZERO))
        .expect("the first strict snapshot is saved");

    checkpoint_without_new_space(&dir, &mut store);

    drop(store);
    unblock_side_files(&dir);
    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), Some(idle()));
}

#[test]
fn checkpoints_after_reopening_a_strict_session_need_no_new_disk_space() {
    let dir = data_dir("in_place_after_reopening");
    let first = strict(Duration::ZERO);
    let mut store = open(&dir);
    store
        .save_snapshot(&first)
        .expect("the first strict snapshot is saved");
    drop(store);

    // The service starts again, as after a reboot, and restores the session.
    let mut store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), Some(first));

    checkpoint_without_new_space(&dir, &mut store);

    drop(store);
    unblock_side_files(&dir);
    let store = open(&dir);
    assert_eq!(store.load_snapshot().expect("reads"), Some(idle()));
}
