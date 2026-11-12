// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.2: SQLite persistence with migrations for
//! blocklists, settings, session history and block events.
//!
//! The engine snapshot, with the full-disk requirements of issue #48, is
//! tested in `acceptance_t2_2_snapshot.rs`.
//!
//! Every test works in its own folder under Cargo's `CARGO_TARGET_TMPDIR`,
//! never in the real `%ProgramData%`. Where a test has to damage or tamper
//! with the database, it closes the store and edits the file directly with
//! `rusqlite`, as a standard user can until T-5.3 sets the folder's ACL.
//!
//! # Expected public API
//!
//! A new module `keitora_service::store`:
//!
//! - `DB_FILE_NAME: &str = "keitora.db"`, the database's file name inside the
//!   data folder (`docs/architecture.md`).
//! - `Store`, the open database. `Debug + Send`.
//!   - `Store::open(data_dir: &Path) -> Result<Store, StoreError>` opens
//!     `<data_dir>\keitora.db`, creating the folder (and missing parents) and
//!     the file if they don't exist, and applies every migration
//!     (`rusqlite_migration`, schema version in `PRAGMA user_version`).
//!     Taking `impl AsRef<Path>` instead also works with these tests.
//!   - Blocklists: `put_blocklist(&mut self, &Blocklist) -> Result<(), _>`
//!     (create or replace, keyed by the blocklist's own `id`),
//!     `blocklists(&self) -> Result<Vec<Blocklist>, _>`,
//!     `blocklist(&self, id: &str) -> Result<Option<Blocklist>, _>`,
//!     `delete_blocklist(&mut self, id: &str) -> Result<bool, _>` (`true`
//!     if it existed).
//!   - Settings: `settings(&self) -> Result<Settings, _>` and
//!     `put_settings(&mut self, &Settings) -> Result<(), _>`.
//!   - History: `record_session(&mut self, &SessionRecord) -> Result<(), _>`
//!     and `history(&self, from: SystemTime, to: SystemTime) ->
//!     Result<Vec<SessionRecord>, _>`.
//!   - Block events: `record_block_event(&mut self, &BlockEvent) ->
//!     Result<(), _>` and `block_events(&self, from: SystemTime, to:
//!     SystemTime) -> Result<Vec<BlockEvent>, _>`.
//!   - The engine snapshot: `save_snapshot` and `load_snapshot`, see
//!     `acceptance_t2_2_snapshot.rs`.
//!
//!   Methods that write take `&mut self` and methods that only read take
//!   `&self`, as listed (the tests hold a `mut` binding and would trip
//!   `unused_mut` otherwise).
//! - `Settings`: `Debug + Clone + PartialEq + Default`, with a public field
//!   `default_preset: PomodoroPreset`, whose default is
//!   `PomodoroPreset::default()`. It may get more fields later (it may be
//!   `#[non_exhaustive]`); the tests only build it from `Settings::default()`.
//! - `SessionRecord`: `Debug + Clone + PartialEq`, built with a struct
//!   literal, so with exactly these public fields and not
//!   `#[non_exhaustive]`: `started_at: SystemTime` (wall time),
//!   `blocklist_id: String`, `preset: PomodoroPreset`, `lock: LockKind`,
//!   `outcome: SessionOutcome`, `focus: Duration` (focus time actually
//!   spent).
//! - `SessionOutcome`: `Completed | Stopped` (`Debug + Clone + Copy +
//!   PartialEq`).
//! - `BlockEvent`: `Debug + Clone + PartialEq`, built with a struct literal,
//!   with exactly these public fields: `at: SystemTime` (wall time),
//!   `kind: TargetKind`, `target: String` (the matched rule: a site pattern
//!   or an app's name, never the visited URL).
//! - `TargetKind`: `Site | App` (`Debug + Clone + Copy + PartialEq`).
//! - `StoreError`: a `thiserror` enum, `Debug`, `#[non_exhaustive]` allowed.
//!   The tests match these variants (with `{ .. }` unless a shape is given,
//!   so their fields are free):
//!   - `SchemaTooNew { found, supported }`: the file's `user_version` is
//!     newer than this build's schema; both are integers (any type that
//!     prints as its number);
//!   - `InvalidBlocklist(ValidationError)`: `put_blocklist` refused a
//!     blocklist that fails `Blocklist::validate`; the error is the one
//!     `validate` returns;
//!   - `InvalidStoredBlocklist { .. }`: a blocklist read from the file
//!     fails `Blocklist::validate`;
//!   - `Corrupt { .. }`: stored data that can't be decoded into its type
//!     (settings, a snapshot that isn't valid `EngineState` JSON, ...);
//!   - `SnapshotTooLarge { .. }`: see the snapshot file;
//!   - `Database { .. }`: an SQLite or file error (a file that isn't a
//!     database, a full disk, a file that can't be written, ...).
//!
//!   Other variants may exist (for example to create the folder).
//!
//! `rusqlite` (with `bundled`) must be a direct dependency of
//! `keitora-service`: these tests use it to read and tamper with the file.
//!
//! # Decisions taken for this task (standing authorization)
//!
//! - **Ids**: the store never invents an id. A blocklist's own validated
//!   `id` is its key, compared exactly (case-sensitive), and storing a
//!   blocklist whose id exists replaces it.
//! - **Order**: `blocklists()` lists blocklists in the order they were first
//!   stored; replacing one keeps its place. `history` and `block_events`
//!   return entries whose wall time is in `[from, to)`, oldest first, and
//!   entries with equal times in the order they were recorded.
//! - **Validation on both sides**: `put_blocklist` calls
//!   `Blocklist::validate` and stores nothing on failure (an existing list
//!   with that id stays). Every blocklist read back is validated again; a
//!   bad one is an error for `blocklist(id)` and for the whole
//!   `blocklists()` call (never a silently shorter list), while other
//!   blocklists stay readable by id (T-1.1 review carryover).
//! - **Settings** hold only what is decided: the default preset. Without
//!   stored settings, `settings()` returns `Settings::default()`.
//! - **Wall time** is kept to the millisecond, for any time Windows can
//!   show from 1601-01-01 to 9999-12-31: it is the user's to set
//!   (ADR-0010), so an odd clock must never fail or panic a write. No
//!   `MonotonicInstant` is ever stored (ADR-0010, ADR-0011); the API takes
//!   only `SystemTime`, and the snapshot holds active time.
//! - **Damaged files are kept**: a file that isn't a database, or whose
//!   schema is newer, is refused with an error and left byte for byte as it
//!   was; it is never deleted, reset or downgraded, since resetting would
//!   also erase a strict session.
//! - **Storage form**, needed to tamper in tests and assumed by ADR-0011
//!   (integrity rests on the folder's ACL): everything lives in the one
//!   SQLite file, as plain UTF-8 text (JSON or text columns), with no
//!   compression or encryption. The tamper helpers search every column of
//!   every table for a piece of text, with `PRAGMA ignore_check_constraints`
//!   on, and fail the test if they find none.
//!
//! # Criteria covered
//!
//! - **ST1** — opening creates the database and applies the migrations, and
//!   records the schema version:
//!   `opening_an_empty_folder_creates_keitora_db_and_records_a_schema_version`,
//!   `opening_a_missing_folder_creates_it`,
//!   `an_empty_database_file_is_migrated_like_a_new_one`,
//!   `the_store_can_be_moved_to_another_thread`.
//! - **ST2** — reopening is idempotent and keeps every kind of data:
//!   `reopening_keeps_every_kind_of_data_and_the_schema_version`.
//! - **ST3** — a database from a newer schema is refused with a typed error
//!   and left untouched; so is a nonsensical (negative) version:
//!   `a_database_from_a_newer_version_is_refused_and_left_untouched`,
//!   `a_negative_schema_version_is_refused_and_left_untouched`.
//! - **ST4** — blocklists round-trip (create, replace, list, get, delete):
//!   `a_stored_blocklist_reads_back_identical`,
//!   `a_blocklist_at_every_maximum_reads_back_identical`,
//!   `blocklists_are_listed_in_the_order_they_were_first_stored`,
//!   `storing_a_blocklist_with_an_existing_id_replaces_it`,
//!   `deleting_a_blocklist_removes_only_that_one`,
//!   `blocklist_ids_are_compared_exactly`.
//! - **ST5** — blocklists are validated before storing and when loading:
//!   `an_invalid_blocklist_is_refused_and_nothing_is_stored`,
//!   `an_invalid_blocklist_doesnt_replace_the_valid_one_with_its_id`,
//!   `a_blocklist_with_a_bad_color_on_disk_is_reported_not_returned`,
//!   `a_blocklist_with_an_unusable_site_pattern_on_disk_is_reported_not_returned`.
//! - **ST6** — settings persist, with defaults when none are stored, and are
//!   checked when read:
//!   `settings_are_the_defaults_until_some_are_stored`,
//!   `stored_settings_read_back_identical_after_reopening`,
//!   `settings_made_invalid_on_disk_are_reported_not_returned`.
//! - **ST7** — finished and stopped sessions are recorded and listed in
//!   order by wall time:
//!   `recorded_sessions_are_listed_by_start_time_within_the_range`,
//!   `history_keeps_any_wall_time_windows_can_show`.
//! - **ST8** — block events are appended and listed by time:
//!   `block_events_are_listed_by_time_within_the_range`,
//!   `block_events_keep_any_wall_time_windows_can_show`.
//! - **ST9** — damaged files give typed errors, never a panic or wrong data,
//!   and are never replaced:
//!   `a_file_that_isnt_a_database_is_refused_and_left_untouched`,
//!   `a_truncated_database_never_returns_wrong_data`.
//! - **ST10** (T-1.1 review, I3) — the store never logs blocklist contents
//!   or block targets, at any level, nor shows them in the `Debug` output of
//!   the store or of its errors:
//!   `the_store_never_logs_blocklist_contents_or_block_targets`.
//!
//! # Verified by review, not here
//!
//! - Migrations use `rusqlite_migration`; each later schema change is a new
//!   migration, never an edit of a released one.
//! - Errors from SQLite never carry stored values into logs (bound
//!   parameters, never values formatted into SQL text, which also rules out
//!   SQL injection through names or patterns).
//! - The data folder is writable by standard users until T-5.3 (the store
//!   adopts a planted file, as `an_empty_database_file_is_migrated_like_a_new_one`
//!   shows); fixing that is T-5.3's, not this task's.
//! - Block events and history grow without limit; pruning is for a later
//!   task.

#![cfg(test)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keitora_core::engine::{LockKind, PomodoroPreset};
use keitora_core::model::{
    AppIdentifiers, Blocklist, MAX_APP_ID_LEN, MAX_EXCEPTIONS, MAX_ID_LEN, MAX_NAME_LEN,
    MAX_PATTERN_LEN, MAX_TARGETS, SitePattern, Target, ValidationErrorKind,
};
use keitora_service::store::{
    BlockEvent, DB_FILE_NAME, SessionOutcome, SessionRecord, Settings, Store, StoreError,
    TargetKind,
};
use rusqlite::{Connection, OpenFlags};

// ---------------------------------------------------------------------------
// Helpers: folders and the raw database file
// ---------------------------------------------------------------------------

/// A fresh, empty data folder for one test, named after it, under Cargo's
/// temporary folder for integration tests.
fn data_dir(test_name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("acceptance_t2_2")
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

/// The database file in `dir`.
fn db_path(dir: &Path) -> PathBuf {
    dir.join(DB_FILE_NAME)
}

/// Opens the store in `dir`, failing the test if it can't.
fn open(dir: &Path) -> Store {
    Store::open(dir).unwrap_or_else(|error| panic!("couldn't open the store: {error:?}"))
}

/// A direct connection to the database file in `dir`, which must exist.
/// Use it only while no `Store` is open on the file.
fn raw_connection(dir: &Path) -> Connection {
    Connection::open_with_flags(db_path(dir), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .expect("couldn't open the database file directly")
}

/// The file's `PRAGMA user_version`.
fn user_version(dir: &Path) -> i64 {
    raw_connection(dir)
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("couldn't read the schema version")
}

/// Sets the file's `PRAGMA user_version`.
fn set_user_version(dir: &Path, version: i64) {
    raw_connection(dir)
        .execute_batch(&format!("PRAGMA user_version = {version};"))
        .expect("couldn't set the schema version");
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

/// Replaces `from` with `to` inside every stored value that contains it, in
/// every column of every table, keeping each value's type (text or blob).
/// Fails the test if no value contained `from`.
fn replace_stored_text(dir: &Path, from: &str, to: &str) {
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
               WHEN 'blob' THEN CAST(replace(CAST({column} AS TEXT), ?1, ?2) AS BLOB) \
               ELSE replace({column}, ?1, ?2) END \
             WHERE instr(CAST({column} AS BLOB), CAST(?1 AS BLOB)) > 0"
        );
        let rows = connection
            .execute(&sql, [from, to])
            .expect("couldn't tamper with the database");
        changed = changed.checked_add(rows).expect("too many rows");
    }
    assert!(
        changed > 0,
        "test setup: no stored value contains {from:?}; the store must keep its data as plain text"
    );
}

// ---------------------------------------------------------------------------
// Helpers: sample data
// ---------------------------------------------------------------------------

/// A valid blocklist with a site, a wildcard site, an app and an exception.
fn sample_blocklist(id: &str, name: &str, color: &str) -> Blocklist {
    let blocklist = Blocklist {
        id: id.to_owned(),
        name: name.to_owned(),
        color: color.to_owned(),
        targets: vec![
            Target::Site {
                pattern: SitePattern::new("youtube.com"),
            },
            Target::Site {
                pattern: SitePattern::new("*.reddit.com"),
            },
            Target::App {
                name: "Steam".to_owned(),
                ids: AppIdentifiers {
                    windows_exe: Some("steam.exe".to_owned()),
                    windows_path: Some(r"C:\Program Files (x86)\Steam\steam.exe".to_owned()),
                    ..AppIdentifiers::default()
                },
            },
        ],
        exceptions: vec![SitePattern::new("youtube.com/feed/subscriptions")],
        block_during_breaks: true,
    };
    blocklist
        .validate()
        .expect("test setup: the sample is valid");
    blocklist
}

/// `text` padded with `fill` between `prefix` and `suffix` to exactly `len`
/// bytes.
fn padded(prefix: &str, fill: &str, suffix: &str, len: usize) -> String {
    let used = prefix
        .len()
        .checked_add(suffix.len())
        .expect("short strings");
    let count = len.checked_sub(used).expect("test setup: prefix too long");
    format!("{prefix}{}{suffix}", fill.repeat(count))
}

/// A valid blocklist at every limit of `keitora_core::model`: the longest
/// id and name, the most targets and exceptions, every pattern and app
/// identifier at its longest.
fn largest_blocklist() -> Blocklist {
    // A family emoji, joined with ZWJ (U+200D): 18 bytes of UTF-8.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let sites = MAX_TARGETS.checked_div(2).expect("non-zero");
    let mut targets = Vec::new();
    for i in 0..MAX_TARGETS {
        if i < sites {
            targets.push(Target::Site {
                pattern: SitePattern::new(padded(
                    &format!("s{i}.example.com/"),
                    "p",
                    "",
                    MAX_PATTERN_LEN,
                )),
            });
        } else {
            targets.push(Target::App {
                name: padded(&format!("App {i} "), "n", "", MAX_NAME_LEN),
                ids: AppIdentifiers {
                    windows_exe: Some(padded(&format!("{i}"), "e", ".exe", MAX_APP_ID_LEN)),
                    windows_path: Some(padded(
                        &format!(r"C:\Apps\{i}\"),
                        "a",
                        ".exe",
                        MAX_APP_ID_LEN,
                    )),
                    android_package: Some(padded("com.example.", "x", "", MAX_APP_ID_LEN)),
                    ios_token: Some(padded("token-", "t", "", MAX_APP_ID_LEN)),
                },
            });
        }
    }
    let exceptions = (0..MAX_EXCEPTIONS)
        .map(|i| {
            SitePattern::new(padded(
                &format!("e{i}.example.org/"),
                "q",
                "",
                MAX_PATTERN_LEN,
            ))
        })
        .collect();
    let blocklist = Blocklist {
        id: padded("big-", "a", "", MAX_ID_LEN),
        name: padded(family, "n", "", MAX_NAME_LEN),
        color: "#AbCdEf".to_owned(),
        targets,
        exceptions,
        block_during_breaks: false,
    };
    blocklist
        .validate()
        .expect("test setup: the largest blocklist is valid");
    blocklist
}

/// The default settings with `preset` as the default preset. Built from
/// `Settings::default()`, so that `Settings` may gain fields or be
/// `#[non_exhaustive]`.
fn settings_with(preset: PomodoroPreset) -> Settings {
    let mut settings = Settings::default();
    settings.default_preset.clone_from(&preset);
    settings
}

/// A wall time `ms` milliseconds after the Unix epoch.
fn at_ms(ms: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_millis(ms))
        .expect("a representable time")
}

/// 2026-11-11T08:00:00.000Z, a base time for history and events.
const BASE_MS: u64 = 1_794_384_000_000;

/// One hour in milliseconds.
const HOUR_MS: u64 = 3_600_000;

/// The earliest time Windows can hold: 1601-01-01T00:00:00Z.
fn earliest_windows_time() -> SystemTime {
    UNIX_EPOCH
        .checked_sub(Duration::from_secs(11_644_473_600))
        .expect("1601 is representable on Windows")
}

/// 9999-12-31T23:59:59.999Z.
fn last_ms_of_year_9999() -> SystemTime {
    at_ms(253_402_300_799_999)
}

/// A session record.
fn session(
    started_at: SystemTime,
    lock: LockKind,
    outcome: SessionOutcome,
    focus: Duration,
) -> SessionRecord {
    SessionRecord {
        started_at,
        blocklist_id: "deep-work".to_owned(),
        preset: PomodoroPreset::new(50, 10, 30, 2).expect("a valid preset"),
        lock,
        outcome,
        focus,
    }
}

/// A block event.
fn event(at: SystemTime, kind: TargetKind, target: &str) -> BlockEvent {
    BlockEvent {
        at,
        kind,
        target: target.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// ST1, ST2, ST3 — opening, migrations and the schema version
// ---------------------------------------------------------------------------

#[test]
fn opening_an_empty_folder_creates_keitora_db_and_records_a_schema_version() {
    assert_eq!(DB_FILE_NAME, "keitora.db");
    let dir = data_dir("creates_db");

    let store = open(&dir);
    assert_eq!(store.blocklists().expect("lists"), Vec::new());
    drop(store);

    assert!(db_path(&dir).is_file(), "keitora.db wasn't created");
    assert!(
        user_version(&dir) >= 1,
        "the migrations must record the schema version in user_version"
    );
}

#[test]
fn opening_a_missing_folder_creates_it() {
    let dir = data_dir("missing_folder").join("Keitora").join("nested");
    assert!(!dir.exists());

    let mut store = open(&dir);
    store
        .put_blocklist(&sample_blocklist("a", "A", "#112233"))
        .expect("stores");
    drop(store);

    assert!(db_path(&dir).is_file());
}

#[test]
fn an_empty_database_file_is_migrated_like_a_new_one() {
    let dir = data_dir("empty_file");
    fs::write(db_path(&dir), b"").expect("test setup");

    let mut store = open(&dir);
    store
        .put_blocklist(&sample_blocklist("a", "A", "#112233"))
        .expect("stores");
    assert_eq!(store.settings().expect("reads"), Settings::default());
    drop(store);

    assert!(user_version(&dir) >= 1);
}

#[test]
fn the_store_can_be_moved_to_another_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<Store>();

    let dir = data_dir("send");
    let mut store = open(&dir);
    let handle = thread::spawn(move || {
        store
            .put_blocklist(&sample_blocklist("a", "A", "#112233"))
            .expect("stores");
        store.blocklists().expect("lists").len()
    });
    assert_eq!(handle.join().expect("the thread doesn't panic"), 1);
}

#[test]
fn reopening_keeps_every_kind_of_data_and_the_schema_version() {
    let dir = data_dir("reopen");
    let lists = vec![
        sample_blocklist("first", "First", "#112233"),
        sample_blocklist("second", "Second", "#445566"),
    ];
    let settings = settings_with(PomodoroPreset::new(45, 10, 20, 3).expect("valid"));
    let record = session(
        at_ms(BASE_MS),
        LockKind::UntilTimerEnds,
        SessionOutcome::Completed,
        Duration::from_secs(45 * 60 * 3),
    );
    let blocked = event(at_ms(BASE_MS + 1_000), TargetKind::Site, "youtube.com");

    let mut store = open(&dir);
    for list in &lists {
        store.put_blocklist(list).expect("stores");
    }
    store.put_settings(&settings).expect("stores");
    store.record_session(&record).expect("records");
    store.record_block_event(&blocked).expect("records");
    drop(store);
    let version = user_version(&dir);

    for _ in 0..3 {
        let store = open(&dir);
        assert_eq!(store.blocklists().expect("lists"), lists);
        assert_eq!(store.settings().expect("reads"), settings);
        assert_eq!(
            store
                .history(at_ms(0), at_ms(BASE_MS + HOUR_MS))
                .expect("lists"),
            vec![record.clone()]
        );
        assert_eq!(
            store
                .block_events(at_ms(0), at_ms(BASE_MS + HOUR_MS))
                .expect("lists"),
            vec![blocked.clone()]
        );
        drop(store);
        assert_eq!(
            user_version(&dir),
            version,
            "reopening changed the schema version"
        );
    }
}

#[test]
fn a_database_from_a_newer_version_is_refused_and_left_untouched() {
    let dir = data_dir("newer_schema");
    let mut store = open(&dir);
    store
        .put_blocklist(&sample_blocklist("kept", "Kept", "#112233"))
        .expect("stores");
    drop(store);
    let current = user_version(&dir);
    let newer = current + 1;
    set_user_version(&dir, newer);
    let before = fs::read(db_path(&dir)).expect("reads the file");

    match Store::open(&dir) {
        Err(StoreError::SchemaTooNew { found, supported }) => {
            // Compared as text, so that any integer type works.
            assert_eq!(found.to_string(), newer.to_string());
            assert_eq!(supported.to_string(), current.to_string());
        }
        other => panic!("expected StoreError::SchemaTooNew, got {other:?}"),
    }

    assert_eq!(
        fs::read(db_path(&dir)).expect("reads the file"),
        before,
        "a newer database must not be modified, migrated or downgraded"
    );
    assert_eq!(user_version(&dir), newer);

    // Nothing was lost: with the version back, the data is still there.
    set_user_version(&dir, current);
    let store = open(&dir);
    assert_eq!(
        store.blocklists().expect("lists"),
        vec![sample_blocklist("kept", "Kept", "#112233")]
    );
}

#[test]
fn a_negative_schema_version_is_refused_and_left_untouched() {
    let dir = data_dir("negative_schema");
    drop(open(&dir));
    set_user_version(&dir, -1);
    let before = fs::read(db_path(&dir)).expect("reads the file");

    let result = Store::open(&dir);
    assert!(
        result.is_err(),
        "a negative schema version must be refused, not read as 0 and migrated: {result:?}"
    );
    drop(result);

    assert_eq!(fs::read(db_path(&dir)).expect("reads the file"), before);
    assert_eq!(user_version(&dir), -1);
}

// ---------------------------------------------------------------------------
// ST4 — blocklists round-trip
// ---------------------------------------------------------------------------

#[test]
fn a_stored_blocklist_reads_back_identical() {
    let dir = data_dir("blocklist_round_trip");
    let list = sample_blocklist(
        "3f0c6b1e-8a52-4c1d-9e2b-7d4a1f6c0b9e",
        "Trabajo profundo \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} \"quoted\" ' ; DROP TABLE x; --",
        "#e5484d",
    );
    let mut store = open(&dir);

    assert_eq!(store.blocklist(&list.id).expect("reads"), None);
    store.put_blocklist(&list).expect("stores");

    assert_eq!(
        store.blocklist(&list.id).expect("reads"),
        Some(list.clone())
    );
    assert_eq!(store.blocklists().expect("lists"), vec![list.clone()]);
    assert_eq!(store.blocklist("unknown").expect("reads"), None);
    drop(store);

    let store = open(&dir);
    assert_eq!(store.blocklist(&list.id).expect("reads"), Some(list));
}

#[test]
fn a_blocklist_at_every_maximum_reads_back_identical() {
    let dir = data_dir("largest_blocklist");
    let list = largest_blocklist();
    let mut store = open(&dir);
    store.put_blocklist(&list).expect("stores");
    drop(store);

    let store = open(&dir);
    assert_eq!(
        store.blocklist(&list.id).expect("reads"),
        Some(list.clone())
    );
    assert_eq!(store.blocklists().expect("lists"), vec![list]);
}

#[test]
fn blocklists_are_listed_in_the_order_they_were_first_stored() {
    let dir = data_dir("blocklist_order");
    let c = sample_blocklist("c", "Charlie", "#111111");
    let a = sample_blocklist("a", "Alpha", "#222222");
    let b = sample_blocklist("b", "Bravo", "#333333");
    let mut store = open(&dir);
    for list in [&c, &a, &b] {
        store.put_blocklist(list).expect("stores");
    }

    assert_eq!(
        store.blocklists().expect("lists"),
        vec![c.clone(), a.clone(), b.clone()]
    );
    drop(store);
    let store = open(&dir);
    assert_eq!(store.blocklists().expect("lists"), vec![c, a, b]);
}

#[test]
fn storing_a_blocklist_with_an_existing_id_replaces_it() {
    let dir = data_dir("blocklist_replace");
    let first = sample_blocklist("first", "First", "#111111");
    let middle = sample_blocklist("middle", "Middle", "#222222");
    let last = sample_blocklist("last", "Last", "#333333");
    let mut store = open(&dir);
    for list in [&first, &middle, &last] {
        store.put_blocklist(list).expect("stores");
    }

    let mut renamed = middle.clone();
    renamed.name = "Renamed".to_owned();
    renamed.targets.truncate(1);
    renamed.exceptions.clear();
    renamed.block_during_breaks = false;
    store.put_blocklist(&renamed).expect("replaces");

    assert_eq!(
        store.blocklist("middle").expect("reads"),
        Some(renamed.clone())
    );
    assert_eq!(
        store.blocklists().expect("lists"),
        vec![first.clone(), renamed.clone(), last.clone()],
        "replacing a blocklist keeps its place and doesn't duplicate it"
    );
    drop(store);
    let store = open(&dir);
    assert_eq!(
        store.blocklists().expect("lists"),
        vec![first, renamed, last]
    );
}

#[test]
fn deleting_a_blocklist_removes_only_that_one() {
    let dir = data_dir("blocklist_delete");
    let a = sample_blocklist("a", "Alpha", "#111111");
    let b = sample_blocklist("b", "Bravo", "#222222");
    let c = sample_blocklist("c", "Charlie", "#333333");
    let mut store = open(&dir);
    for list in [&a, &b, &c] {
        store.put_blocklist(list).expect("stores");
    }

    assert!(store.delete_blocklist("b").expect("deletes"));
    assert_eq!(store.blocklist("b").expect("reads"), None);
    assert_eq!(
        store.blocklists().expect("lists"),
        vec![a.clone(), c.clone()]
    );

    assert!(
        !store.delete_blocklist("b").expect("deletes"),
        "deleting a missing blocklist reports false"
    );
    assert!(!store.delete_blocklist("never-stored").expect("deletes"));
    assert!(!store.delete_blocklist("not an id!").expect("deletes"));

    // Stored again, it goes to the end.
    store.put_blocklist(&b).expect("stores");
    assert_eq!(
        store.blocklists().expect("lists"),
        vec![a.clone(), c.clone(), b.clone()]
    );
    drop(store);
    let store = open(&dir);
    assert_eq!(store.blocklists().expect("lists"), vec![a, c, b]);
}

#[test]
fn blocklist_ids_are_compared_exactly() {
    let dir = data_dir("blocklist_id_case");
    let lower = sample_blocklist("abc", "Lower", "#111111");
    let upper = sample_blocklist("ABC", "Upper", "#222222");
    let mut store = open(&dir);
    store.put_blocklist(&lower).expect("stores");
    store.put_blocklist(&upper).expect("stores");

    assert_eq!(
        store.blocklists().expect("lists"),
        vec![lower.clone(), upper.clone()]
    );
    assert_eq!(store.blocklist("abc").expect("reads"), Some(lower.clone()));
    assert_eq!(store.blocklist("ABC").expect("reads"), Some(upper));
    assert_eq!(store.blocklist("Abc").expect("reads"), None);

    assert!(store.delete_blocklist("ABC").expect("deletes"));
    assert_eq!(store.blocklists().expect("lists"), vec![lower]);
}

// ---------------------------------------------------------------------------
// ST5 — validation before storing and when loading
// ---------------------------------------------------------------------------

#[test]
fn an_invalid_blocklist_is_refused_and_nothing_is_stored() {
    let dir = data_dir("invalid_blocklist");
    let mut store = open(&dir);

    let mut bad_color = sample_blocklist("bad-color", "Bad color", "#112233");
    bad_color.color = "red".to_owned();
    let mut bad_id = sample_blocklist("x", "Bad id", "#112233");
    bad_id.id = "not an id!".to_owned();
    let mut unc_path = sample_blocklist("unc", "UNC path", "#112233");
    unc_path.targets = vec![Target::App {
        name: "Remote".to_owned(),
        ids: AppIdentifiers {
            windows_path: Some(r"\\attacker\share\x.exe".to_owned()),
            ..AppIdentifiers::default()
        },
    }];
    let mut too_many = sample_blocklist("too-many", "Too many", "#112233");
    too_many.targets = (0..=MAX_TARGETS)
        .map(|i| Target::Site {
            pattern: SitePattern::new(format!("s{i}.example.com")),
        })
        .collect();
    let mut bad_pattern = sample_blocklist("bad-pattern", "Bad pattern", "#112233");
    bad_pattern.exceptions = vec![SitePattern::new("https://example.com:8080/")];

    for invalid in [bad_color, bad_id, unc_path, too_many, bad_pattern] {
        let expected = invalid
            .validate()
            .expect_err("test setup: the blocklist is invalid");
        match store.put_blocklist(&invalid) {
            Err(StoreError::InvalidBlocklist(error)) => assert_eq!(error, expected),
            other => panic!(
                "expected StoreError::InvalidBlocklist for {}, got {other:?}",
                invalid.id
            ),
        }
        assert_eq!(store.blocklist(&invalid.id).expect("reads"), None);
    }
    assert_eq!(store.blocklists().expect("lists"), Vec::new());
    drop(store);

    let store = open(&dir);
    assert_eq!(store.blocklists().expect("lists"), Vec::new());
}

#[test]
fn an_invalid_blocklist_doesnt_replace_the_valid_one_with_its_id() {
    let dir = data_dir("invalid_doesnt_replace");
    let valid = sample_blocklist("work", "Work", "#112233");
    let mut store = open(&dir);
    store.put_blocklist(&valid).expect("stores");

    let mut invalid = valid.clone();
    invalid.name = "Line\nbreak".to_owned();
    let result = store.put_blocklist(&invalid);
    assert!(
        matches!(result, Err(StoreError::InvalidBlocklist(_))),
        "expected StoreError::InvalidBlocklist, got {result:?}"
    );

    assert_eq!(store.blocklist("work").expect("reads"), Some(valid.clone()));
    drop(store);
    let store = open(&dir);
    assert_eq!(store.blocklists().expect("lists"), vec![valid]);
}

#[test]
fn a_blocklist_with_a_bad_color_on_disk_is_reported_not_returned() {
    let dir = data_dir("tampered_color");
    let good = sample_blocklist("good", "Good", "#2468ac");
    let tampered = sample_blocklist("tampered", "Tampered", "#13579b");
    let mut store = open(&dir);
    store.put_blocklist(&good).expect("stores");
    store.put_blocklist(&tampered).expect("stores");
    drop(store);

    // Still valid JSON and valid text, but not a color: only
    // `Blocklist::validate` can catch it.
    replace_stored_text(&dir, "#13579b", "#13579g");
    assert_eq!(
        tampered_with_color("#13579g")
            .validate()
            .map_err(|e| e.kind()),
        Err(ValidationErrorKind::NotAColor),
        "test setup"
    );

    let store = open(&dir);
    let by_id = store.blocklist("tampered");
    assert!(
        matches!(by_id, Err(StoreError::InvalidStoredBlocklist { .. })),
        "expected StoreError::InvalidStoredBlocklist, got {by_id:?}"
    );
    let all = store.blocklists();
    assert!(
        matches!(all, Err(StoreError::InvalidStoredBlocklist { .. })),
        "listing must fail rather than return the tampered blocklist or silently skip it, got {all:?}"
    );
    assert_eq!(
        store
            .blocklist("good")
            .expect("an untouched blocklist still reads"),
        Some(good)
    );
}

/// The sample blocklist `tampered` with another color, unchecked.
fn tampered_with_color(color: &str) -> Blocklist {
    let mut list = sample_blocklist("tampered", "Tampered", "#13579b");
    color.clone_into(&mut list.color);
    list
}

#[test]
fn a_blocklist_with_an_unusable_site_pattern_on_disk_is_reported_not_returned() {
    let dir = data_dir("tampered_pattern");
    let mut list = sample_blocklist("tampered", "Tampered", "#112233");
    list.targets.push(Target::Site {
        pattern: SitePattern::new("tamper-me.example"),
    });
    list.validate().expect("test setup");
    let mut store = open(&dir);
    store.put_blocklist(&list).expect("stores");
    drop(store);

    // A port: still valid JSON, but not a pattern the matcher can use.
    replace_stored_text(&dir, "tamper-me.example", "tamper-me.example:8080");

    let store = open(&dir);
    let by_id = store.blocklist("tampered");
    assert!(
        matches!(by_id, Err(StoreError::InvalidStoredBlocklist { .. })),
        "expected StoreError::InvalidStoredBlocklist, got {by_id:?}"
    );
    let all = store.blocklists();
    assert!(
        matches!(all, Err(StoreError::InvalidStoredBlocklist { .. })),
        "expected StoreError::InvalidStoredBlocklist, got {all:?}"
    );
}

// ---------------------------------------------------------------------------
// ST6 — settings
// ---------------------------------------------------------------------------

#[test]
fn settings_are_the_defaults_until_some_are_stored() {
    assert_eq!(
        Settings::default().default_preset,
        PomodoroPreset::default()
    );

    let dir = data_dir("default_settings");
    let store = open(&dir);
    assert_eq!(store.settings().expect("reads"), Settings::default());
    drop(store);
    let store = open(&dir);
    assert_eq!(store.settings().expect("reads"), Settings::default());
}

#[test]
fn stored_settings_read_back_identical_after_reopening() {
    let dir = data_dir("settings_round_trip");
    let first = settings_with(PomodoroPreset::new(120, 30, 60, 8).expect("valid"));
    let second = settings_with(PomodoroPreset::new(1, 1, 1, 1).expect("valid"));

    let mut store = open(&dir);
    store.put_settings(&first).expect("stores");
    assert_eq!(store.settings().expect("reads"), first);
    store.put_settings(&second).expect("replaces");
    assert_eq!(store.settings().expect("reads"), second);
    drop(store);

    let store = open(&dir);
    assert_eq!(store.settings().expect("reads"), second);
}

#[test]
fn settings_made_invalid_on_disk_are_reported_not_returned() {
    let dir = data_dir("tampered_settings");
    // 37 minutes appears nowhere else in the file.
    let settings = settings_with(PomodoroPreset::new(37, 5, 15, 4).expect("valid"));
    let mut store = open(&dir);
    store.put_settings(&settings).expect("stores");
    drop(store);

    // 999 minutes of focus is outside `PomodoroPreset::new`'s range.
    replace_stored_text(&dir, "37", "999");

    let store = open(&dir);
    let read = store.settings();
    assert!(
        matches!(read, Err(StoreError::Corrupt { .. })),
        "expected StoreError::Corrupt, never the defaults or an unchecked preset, got {read:?}"
    );
}

// ---------------------------------------------------------------------------
// ST7 — session history
// ---------------------------------------------------------------------------

#[test]
fn recorded_sessions_are_listed_by_start_time_within_the_range() {
    let dir = data_dir("history_order");
    let from = at_ms(BASE_MS);
    let to = at_ms(BASE_MS + 3 * HOUR_MS);

    let before_range = session(
        at_ms(BASE_MS - 1),
        LockKind::None,
        SessionOutcome::Completed,
        Duration::from_secs(25 * 60),
    );
    let third = session(
        at_ms(BASE_MS + 2 * HOUR_MS),
        LockKind::UntilTimerEnds,
        SessionOutcome::Completed,
        Duration::from_secs(100 * 60),
    );
    let first = session(
        from,
        LockKind::RandomText { len: 12 },
        SessionOutcome::Stopped,
        Duration::from_millis(7 * 60 * 1000 + 30_250),
    );
    let second = session(
        at_ms(BASE_MS + HOUR_MS),
        LockKind::None,
        SessionOutcome::Stopped,
        Duration::ZERO,
    );
    let second_tie = session(
        at_ms(BASE_MS + HOUR_MS),
        LockKind::UntilTimerEnds,
        SessionOutcome::Completed,
        Duration::from_secs(50 * 60),
    );
    let at_end = session(
        to,
        LockKind::None,
        SessionOutcome::Completed,
        Duration::from_secs(60),
    );

    let mut store = open(&dir);
    // Recorded out of order, as when the user moves the clock back.
    for record in [&third, &at_end, &first, &before_range, &second, &second_tie] {
        store.record_session(record).expect("records");
    }

    let expected = vec![
        first.clone(),
        second.clone(),
        second_tie.clone(),
        third.clone(),
    ];
    assert_eq!(store.history(from, to).expect("lists"), expected);
    assert_eq!(
        store
            .history(to, at_ms(BASE_MS + 4 * HOUR_MS))
            .expect("lists"),
        vec![at_end.clone()]
    );
    assert_eq!(store.history(to, to).expect("lists"), Vec::new());
    assert_eq!(store.history(to, from).expect("lists"), Vec::new());
    drop(store);

    let store = open(&dir);
    assert_eq!(store.history(from, to).expect("lists"), expected);
    assert_eq!(
        store
            .history(earliest_windows_time(), last_ms_of_year_9999())
            .expect("lists"),
        vec![before_range, first, second, second_tie, third, at_end]
    );
}

#[test]
fn history_keeps_any_wall_time_windows_can_show() {
    let dir = data_dir("history_wall_times");
    let earliest = session(
        earliest_windows_time(),
        LockKind::None,
        SessionOutcome::Stopped,
        Duration::from_secs(1),
    );
    let epoch = session(
        UNIX_EPOCH,
        LockKind::None,
        SessionOutcome::Completed,
        Duration::from_secs(2),
    );
    let latest = session(
        last_ms_of_year_9999(),
        LockKind::UntilTimerEnds,
        SessionOutcome::Completed,
        Duration::from_millis(1_230 * 60 * 1000),
    );

    let mut store = open(&dir);
    for record in [&latest, &epoch, &earliest] {
        store
            .record_session(record)
            .expect("any wall time is recorded");
    }
    drop(store);

    let store = open(&dir);
    let end = last_ms_of_year_9999() + Duration::from_millis(1);
    assert_eq!(
        store.history(earliest_windows_time(), end).expect("lists"),
        vec![earliest, epoch, latest]
    );
}

// ---------------------------------------------------------------------------
// ST8 — block events
// ---------------------------------------------------------------------------

#[test]
fn block_events_are_listed_by_time_within_the_range() {
    let dir = data_dir("block_events");
    let from = at_ms(BASE_MS);
    let to = at_ms(BASE_MS + HOUR_MS);

    let before = event(at_ms(BASE_MS - 1), TargetKind::Site, "youtube.com");
    let first = event(from, TargetKind::App, "Steam");
    let second = event(at_ms(BASE_MS + 500), TargetKind::Site, "*.reddit.com");
    let second_tie = event(at_ms(BASE_MS + 500), TargetKind::Site, "youtube.com/shorts");
    let last = event(at_ms(BASE_MS + HOUR_MS - 1), TargetKind::App, "Discord");
    let at_end = event(to, TargetKind::Site, "x.com");

    let mut store = open(&dir);
    for blocked in [&last, &at_end, &second, &before, &first, &second_tie] {
        store.record_block_event(blocked).expect("records");
    }

    let expected = vec![
        first.clone(),
        second.clone(),
        second_tie.clone(),
        last.clone(),
    ];
    assert_eq!(store.block_events(from, to).expect("lists"), expected);
    assert_eq!(store.block_events(to, to).expect("lists"), Vec::new());
    assert_eq!(store.block_events(to, from).expect("lists"), Vec::new());
    drop(store);

    let store = open(&dir);
    assert_eq!(store.block_events(from, to).expect("lists"), expected);
    assert_eq!(
        store
            .block_events(earliest_windows_time(), last_ms_of_year_9999())
            .expect("lists"),
        vec![before, first, second, second_tie, last, at_end]
    );
}

#[test]
fn block_events_keep_any_wall_time_windows_can_show() {
    let dir = data_dir("block_event_wall_times");
    let earliest = event(earliest_windows_time(), TargetKind::App, "Steam");
    let latest = event(last_ms_of_year_9999(), TargetKind::Site, "youtube.com");

    let mut store = open(&dir);
    store.record_block_event(&latest).expect("records");
    store.record_block_event(&earliest).expect("records");
    drop(store);

    let store = open(&dir);
    let end = last_ms_of_year_9999() + Duration::from_millis(1);
    assert_eq!(
        store
            .block_events(earliest_windows_time(), end)
            .expect("lists"),
        vec![earliest, latest]
    );
}

// ---------------------------------------------------------------------------
// ST9 — damaged files
// ---------------------------------------------------------------------------

#[test]
fn a_file_that_isnt_a_database_is_refused_and_left_untouched() {
    let not_sqlite = "This is not a database.\r\n".repeat(400).into_bytes();
    let mut bad_header = b"SQLite format 3\0".to_vec();
    bad_header.extend(std::iter::repeat_n(0xA5_u8, 8192));

    for (name, content) in [("text_file", not_sqlite), ("bad_header", bad_header)] {
        let dir = data_dir(&format!("not_a_database_{name}"));
        fs::write(db_path(&dir), &content).expect("test setup");

        let result = Store::open(&dir);
        assert!(
            matches!(
                result,
                Err(StoreError::Database { .. } | StoreError::Corrupt { .. })
            ),
            "{name}: expected StoreError::Database or StoreError::Corrupt, got {result:?}"
        );
        drop(result);

        assert_eq!(
            fs::read(db_path(&dir)).expect("reads the file"),
            content,
            "{name}: the store must never replace or reset a file it can't read"
        );
    }
}

#[test]
fn a_truncated_database_never_returns_wrong_data() {
    let dir = data_dir("truncated");
    let lists: Vec<Blocklist> = (0..4)
        .map(|i| {
            let mut list = largest_blocklist();
            list.id = format!("list-{i}");
            list
        })
        .collect();
    let settings = settings_with(PomodoroPreset::new(50, 10, 30, 2).expect("valid"));
    let record = session(
        at_ms(BASE_MS),
        LockKind::None,
        SessionOutcome::Completed,
        Duration::from_secs(60),
    );

    let mut store = open(&dir);
    for list in &lists {
        store.put_blocklist(list).expect("stores");
    }
    store.put_settings(&settings).expect("stores");
    store.record_session(&record).expect("records");
    drop(store);

    let file = fs::OpenOptions::new()
        .write(true)
        .open(db_path(&dir))
        .expect("opens the file");
    let len = file.metadata().expect("reads its size").len();
    file.set_len(len / 2).expect("truncates the file");
    drop(file);

    // Opening or any read may fail, with an error; none may panic, and
    // whatever is returned must be what was stored.
    let Ok(store) = Store::open(&dir) else {
        return;
    };
    if let Ok(all) = store.blocklists() {
        assert_eq!(all, lists, "a damaged file returned a wrong list");
    }
    for list in &lists {
        if let Ok(found) = store.blocklist(&list.id) {
            assert_eq!(
                found.as_ref(),
                Some(list),
                "a damaged file returned a wrong blocklist"
            );
        }
    }
    if let Ok(read) = store.settings() {
        assert_eq!(read, settings, "a damaged file returned wrong settings");
    }
    if let Ok(history) = store.history(at_ms(0), at_ms(BASE_MS + HOUR_MS)) {
        assert_eq!(
            history,
            vec![record],
            "a damaged file returned a wrong history"
        );
    }
}

// ---------------------------------------------------------------------------
// ST10 — no user data in logs
// ---------------------------------------------------------------------------

/// Log output captured in memory.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
    }
}

#[test]
fn the_store_never_logs_blocklist_contents_or_block_targets() {
    const MARKERS: [&str; 5] = [
        "MarkerListName",
        "marker-site.example",
        "MarkerAppName",
        "marker-exception.example",
        "marker-blocked-target.example",
    ];
    let dir = data_dir("no_user_data_in_logs");
    let list = Blocklist {
        id: "logged-list".to_owned(),
        name: "MarkerListName".to_owned(),
        color: "#13579b".to_owned(),
        targets: vec![
            Target::Site {
                pattern: SitePattern::new("marker-site.example"),
            },
            Target::App {
                name: "MarkerAppName".to_owned(),
                ids: AppIdentifiers {
                    windows_exe: Some("marker.exe".to_owned()),
                    ..AppIdentifiers::default()
                },
            },
        ],
        exceptions: vec![SitePattern::new("marker-exception.example")],
        block_during_breaks: false,
    };
    list.validate().expect("test setup");
    let mut invalid = list.clone();
    invalid.id = "invalid-list".to_owned();
    invalid.color = "MarkerListName".to_owned();

    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();

    let debug_output = tracing::subscriber::with_default(subscriber, || {
        let mut debug_output = String::new();
        let mut store = open(&dir);
        store.put_blocklist(&list).expect("stores");
        store.blocklist("logged-list").expect("reads");
        store.blocklists().expect("lists");
        // Only errors are formatted: a returned blocklist would contain the
        // markers by design.
        let refused = store.put_blocklist(&invalid).err();
        debug_output.push_str(&format!("{refused:?}"));
        store
            .record_block_event(&event(
                at_ms(BASE_MS),
                TargetKind::Site,
                "marker-blocked-target.example",
            ))
            .expect("records");
        store
            .block_events(at_ms(0), at_ms(BASE_MS + 1))
            .expect("lists");
        debug_output.push_str(&format!("{store:?}"));
        drop(store);

        replace_stored_text(&dir, "#13579b", "#13579g");
        let store = open(&dir);
        let tampered = store.blocklist("logged-list").err();
        debug_output.push_str(&format!("{tampered:?}"));
        let all = store.blocklists().err();
        debug_output.push_str(&format!("{all:?}"));
        debug_output
    });

    let logged = logs.text();
    for marker in MARKERS {
        assert!(
            !logged.contains(marker),
            "the logs contain user data ({marker}):\n{logged}"
        );
        assert!(
            !debug_output.contains(marker),
            "a Debug output contains user data ({marker}):\n{debug_output}"
        );
    }
}
