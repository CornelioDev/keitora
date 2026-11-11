// SPDX-License-Identifier: MIT OR Apache-2.0

//! The service's database: blocklists, settings, session history, block
//! events and the engine snapshot, in one SQLite file,
//! `<data dir>\keitora.db` ([`DB_FILE_NAME`]).
//!
//! [`Store::open`] creates the file if needed and brings its schema up to
//! date with the migrations of `rusqlite_migration`. Every value is stored
//! as plain UTF-8 text or an integer, with no compression or encryption: its
//! integrity rests on who may write the data folder, which T-5.3 restricts
//! to `SYSTEM` and Administrators (ADR-0011).
//!
//! # Stored data is untrusted
//!
//! Until T-5.3, any standard user can edit or replace the file, so the
//! store checks everything it reads as it checks what it is given:
//!
//! - A blocklist passes [`Blocklist::validate`] before it is stored and
//!   again whenever it is read; one that fails on reading is
//!   [`StoreError::InvalidStoredBlocklist`], never skipped or returned.
//! - Settings, presets, locks and the engine snapshot go through the same
//!   checked deserializers as everywhere else; a value that fails is
//!   [`StoreError::Corrupt`], never replaced by a default.
//! - The engine snapshot's length is checked, in SQL, before it is read or
//!   parsed ([`MAX_SNAPSHOT_BYTES`]).
//! - A file that isn't a database, is damaged, or comes from a newer schema
//!   is refused with an error and **never deleted, reset or downgraded**:
//!   that would also erase a strict session.
//!
//! # A full disk can't erase a strict session (#48, ADR-0013)
//!
//! - Opening an up-to-date database writes nothing, so a store whose file
//!   can't be written still opens and reads, and the service can restore a
//!   session; every write then fails with [`StoreError::Database`].
//! - [`Store::save_snapshot`] returns `Ok` only once the snapshot is on
//!   disk (`PRAGMA synchronous = FULL`), and is atomic: on error, the
//!   snapshot stored before is still the one stored.
//! - Once a snapshot has been saved, saving another needs no new disk
//!   space: the snapshot is padded to a fixed length and overwrites the same
//!   page, and the rollback journal is kept between writes
//!   (`journal_mode = PERSIST`) at the size those writes need.
//!
//! # No user data in logs
//!
//! The store never logs a blocklist, a setting or a block event, and neither
//! [`Store`]'s `Debug` nor [`StoreError`] contains one.
//!
//! [`Blocklist::validate`]: keitora_core::model::Blocklist::validate

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use keitora_core::engine::{EngineState, LockKind, PomodoroPreset};
use keitora_core::model::Blocklist;
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

mod error;
mod schema;
mod wall_time;

pub use error::StoreError;

/// The database's file name inside the data folder
/// ([`crate::data_dir`]).
pub const DB_FILE_NAME: &str = "keitora.db";

/// The longest engine snapshot the store saves or loads, in bytes of its
/// JSON.
///
/// The longest snapshot the engine writes is about 250 bytes, so this
/// leaves room for fields later versions may add (ADR-0011). Every snapshot
/// is padded with spaces to exactly this length, which keeps each save in
/// the same place on disk (#48), and fits in one 4 KiB SQLite page, so a
/// save rewrites a single page of the table. A stored value longer than
/// this is refused without being parsed.
pub const MAX_SNAPSHOT_BYTES: usize = 2048;

/// What the service remembers between runs, apart from blocklists.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Settings {
    /// The preset a new session starts with unless the user picks another.
    pub default_preset: PomodoroPreset,
}

/// A session that ended, as kept in the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// When it started, by the wall clock. Kept to the millisecond, rounded
    /// down.
    pub started_at: SystemTime,
    /// The id of the blocklist it used.
    pub blocklist_id: String,
    /// Its durations and cycles.
    pub preset: PomodoroPreset,
    /// How it was locked.
    pub lock: LockKind,
    /// How it ended.
    pub outcome: SessionOutcome,
    /// The focus time actually spent, outside pauses and breaks.
    pub focus: Duration,
}

/// How a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    /// It ran until its last phase ended.
    Completed,
    /// The user stopped it early.
    Stopped,
}

impl SessionOutcome {
    /// Its value in the `outcome` column.
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Stopped => "stopped",
        }
    }

    /// The outcome stored as `text`, if it is one.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "completed" => Some(Self::Completed),
            "stopped" => Some(Self::Stopped),
            _ => None,
        }
    }
}

/// A site or an application that was blocked, as kept in the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEvent {
    /// When it was blocked, by the wall clock. Kept to the millisecond,
    /// rounded down.
    pub at: SystemTime,
    /// Whether a site or an application was blocked.
    pub kind: TargetKind,
    /// The rule that matched: a site pattern or an application's name,
    /// never the visited URL.
    pub target: String,
}

/// Whether a [`BlockEvent`] is about a site or an application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// A website, blocked by the browser extension.
    Site,
    /// An application, blocked by the service.
    App,
}

impl TargetKind {
    /// Its value in the `kind` column.
    fn as_str(self) -> &'static str {
        match self {
            Self::Site => "site",
            Self::App => "app",
        }
    }

    /// The kind stored as `text`, if it is one.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "site" => Some(Self::Site),
            "app" => Some(Self::App),
            _ => None,
        }
    }
}

/// The open database. Writing methods take `&mut self` and reading ones
/// `&self`; it can be moved to another thread, but not shared between
/// threads.
///
/// See the [module documentation](self) for what is stored and how it is
/// checked.
pub struct Store {
    /// The connection to `keitora.db`.
    connection: Connection,
    /// The database file's path, for `Debug`.
    path: PathBuf,
}

impl fmt::Debug for Store {
    /// Shows the file's path only, never stored data.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Opens `<data_dir>\keitora.db`, creating the folder (with any missing
    /// parents) and the file if they don't exist, and applies the
    /// migrations the file lacks.
    ///
    /// An up-to-date database is only read, so this works even when the
    /// file can't be written, as on a full disk.
    ///
    /// # Errors
    ///
    /// - [`StoreError::CreateFolder`] if the folder can't be created.
    /// - [`StoreError::SchemaTooNew`] if the file's schema is newer than this
    ///   build's, and [`StoreError::Corrupt`] if its schema version is
    ///   negative. The file is left as it was.
    /// - [`StoreError::Database`] if the file isn't a database, is damaged,
    ///   or can't be migrated, for instance because it can't be written.
    ///   The file is never deleted or reset.
    /// - [`StoreError::Migration`] if a migration fails for another reason,
    ///   such as an old database that already has a table it creates.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let data_dir = data_dir.as_ref();
        fs::create_dir_all(data_dir).map_err(|source| StoreError::CreateFolder { source })?;
        let path = data_dir.join(DB_FILE_NAME);
        // No `SQLITE_OPEN_URI`: the path is a file name, never a URI with
        // options. If the file is read-only, SQLite opens it for reading.
        let mut connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        // The connection's settings come first. They do no I/O, so they
        // write nothing even to a file refused below, and with them the
        // first read, which rolls back a transaction a crash interrupted,
        // keeps the journal file instead of deleting it.
        configure(&connection)?;

        // The version comes before anything could write: a newer or
        // nonsensical one is refused with the file untouched.
        let found: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if found > schema::SCHEMA_VERSION {
            return Err(StoreError::SchemaTooNew {
                found,
                supported: schema::SCHEMA_VERSION,
            });
        }
        if found < 0 {
            return Err(StoreError::Corrupt {
                what: "schema version",
            });
        }

        // Only now, after a read, is the journal mode the file imposes known.
        let mode: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("persist") {
            // Only a database in WAL mode, which this store never sets, has
            // another mode. Reading still works, and refusing to open would
            // stop the service from restoring a session.
            tracing::warn!(
                journal_mode = mode.as_str(),
                "the database isn't in PERSIST journal mode; checkpoints may need new disk space"
            );
        }

        if found < schema::SCHEMA_VERSION {
            schema::migrations().to_latest(&mut connection)?;
            tracing::info!(
                from = found,
                to = schema::SCHEMA_VERSION,
                "migrated the database schema"
            );
        }
        Ok(Self { connection, path })
    }

    // -----------------------------------------------------------------------
    // Blocklists
    // -----------------------------------------------------------------------

    /// Stores `blocklist`, keyed by its `id`: a new id is added after every
    /// stored blocklist, and an existing one is replaced, keeping its place.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidBlocklist`] if `blocklist` fails
    /// [`Blocklist::validate`]; nothing is stored, and a blocklist stored
    /// with that id stays. [`StoreError::Database`] if it can't be written.
    pub fn put_blocklist(&mut self, blocklist: &Blocklist) -> Result<(), StoreError> {
        blocklist.validate().map_err(StoreError::InvalidBlocklist)?;
        let json = encode(blocklist, "blocklist")?;
        self.connection.execute(
            "INSERT INTO blocklist (id, json) VALUES (?1, ?2) \
             ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            params![blocklist.id, json],
        )?;
        Ok(())
    }

    /// Every stored blocklist, in the order they were first stored.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidStoredBlocklist`] or [`StoreError::Corrupt`] if
    /// any stored blocklist is invalid or can't be decoded: the whole list
    /// fails, it is never returned shorter. [`StoreError::Database`] if the
    /// file can't be read.
    pub fn blocklists(&self) -> Result<Vec<Blocklist>, StoreError> {
        let mut query = self
            .connection
            .prepare("SELECT id, json FROM blocklist ORDER BY position")?;
        let mut rows = query.query([])?;
        let mut blocklists = Vec::new();
        while let Some(row) = rows.next()? {
            blocklists.push(decode_blocklist(row)?);
        }
        Ok(blocklists)
    }

    /// The stored blocklist whose id is exactly `id`, if any.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidStoredBlocklist`] or [`StoreError::Corrupt`] if
    /// it is invalid or can't be decoded. [`StoreError::Database`] if the
    /// file can't be read.
    pub fn blocklist(&self, id: &str) -> Result<Option<Blocklist>, StoreError> {
        let mut query = self
            .connection
            .prepare("SELECT id, json FROM blocklist WHERE id = ?1")?;
        let mut rows = query.query([id])?;
        rows.next()?.map(decode_blocklist).transpose()
    }

    /// Deletes the blocklist whose id is exactly `id`. Returns whether there
    /// was one.
    ///
    /// # Errors
    ///
    /// [`StoreError::Database`] if the file can't be written.
    pub fn delete_blocklist(&mut self, id: &str) -> Result<bool, StoreError> {
        let deleted = self
            .connection
            .execute("DELETE FROM blocklist WHERE id = ?1", [id])?;
        Ok(deleted > 0)
    }

    // -----------------------------------------------------------------------
    // Settings
    // -----------------------------------------------------------------------

    /// The stored settings, or [`Settings::default`] if none were ever
    /// stored.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] if the stored settings can't be decoded or
    /// are out of range: they are never replaced by the defaults.
    /// [`StoreError::Database`] if the file can't be read.
    pub fn settings(&self) -> Result<Settings, StoreError> {
        let json: Option<String> = self
            .connection
            .query_row("SELECT json FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        match json {
            Some(json) => decode(&json, "settings"),
            None => Ok(Settings::default()),
        }
    }

    /// Stores `settings`, replacing the stored ones.
    ///
    /// # Errors
    ///
    /// [`StoreError::Database`] if the file can't be written.
    pub fn put_settings(&mut self, settings: &Settings) -> Result<(), StoreError> {
        let json = encode(settings, "settings")?;
        self.connection.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1) \
             ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            [json],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Session history
    // -----------------------------------------------------------------------

    /// Adds `record` to the history.
    ///
    /// # Errors
    ///
    /// [`StoreError::OutOfRange`] if its start time or focus time doesn't
    /// fit in the database's integers, which no time Windows can hold and
    /// no session's focus reaches. [`StoreError::Database`] if the file
    /// can't be written.
    pub fn record_session(&mut self, record: &SessionRecord) -> Result<(), StoreError> {
        let started_at_ms = wall_time::to_millis(record.started_at)
            .ok_or(StoreError::OutOfRange { what: "wall time" })?;
        let focus_ns = i64::try_from(record.focus.as_nanos())
            .map_err(|_| StoreError::OutOfRange { what: "focus time" })?;
        self.connection.execute(
            "INSERT INTO session_history \
               (started_at_ms, blocklist_id, preset, lock, outcome, focus_ns) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                started_at_ms,
                record.blocklist_id,
                encode(&record.preset, "preset")?,
                encode(&record.lock, "lock")?,
                record.outcome.as_str(),
                focus_ns,
            ],
        )?;
        Ok(())
    }

    /// The sessions that started in `[from, to)`, oldest first; sessions
    /// that started at the same millisecond in the order they were
    /// recorded. Times are compared to the millisecond, each rounded down.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] if a record in the range can't be decoded.
    /// [`StoreError::Database`] if the file can't be read.
    pub fn history(
        &self,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let mut query = self.connection.prepare(
            "SELECT started_at_ms, blocklist_id, preset, lock, outcome, focus_ns \
             FROM session_history \
             WHERE started_at_ms >= ?1 AND started_at_ms < ?2 \
             ORDER BY started_at_ms, seq",
        )?;
        let mut rows = query.query(range(from, to))?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(decode_session(row)?);
        }
        Ok(records)
    }

    // -----------------------------------------------------------------------
    // Block events
    // -----------------------------------------------------------------------

    /// Adds `event` to the block events.
    ///
    /// # Errors
    ///
    /// [`StoreError::OutOfRange`] if its time doesn't fit in the database's
    /// integers, which no time Windows can hold. [`StoreError::Database`]
    /// if the file can't be written.
    pub fn record_block_event(&mut self, event: &BlockEvent) -> Result<(), StoreError> {
        let at_ms =
            wall_time::to_millis(event.at).ok_or(StoreError::OutOfRange { what: "wall time" })?;
        self.connection.execute(
            "INSERT INTO block_event (at_ms, kind, target) VALUES (?1, ?2, ?3)",
            params![at_ms, event.kind.as_str(), event.target],
        )?;
        Ok(())
    }

    /// The block events that happened in `[from, to)`, oldest first; events
    /// at the same millisecond in the order they were recorded. Times are
    /// compared to the millisecond, each rounded down.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] if an event in the range can't be decoded.
    /// [`StoreError::Database`] if the file can't be read.
    pub fn block_events(
        &self,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<Vec<BlockEvent>, StoreError> {
        let mut query = self.connection.prepare(
            "SELECT at_ms, kind, target FROM block_event \
             WHERE at_ms >= ?1 AND at_ms < ?2 \
             ORDER BY at_ms, seq",
        )?;
        let mut rows = query.query(range(from, to))?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(decode_block_event(row)?);
        }
        Ok(events)
    }

    // -----------------------------------------------------------------------
    // The engine snapshot
    // -----------------------------------------------------------------------

    /// Stores `state` as the engine snapshot, replacing the stored one.
    ///
    /// `Ok` means it is on disk, flushed. The write is atomic: on error, the
    /// snapshot stored before is still the one stored, whole. Once any
    /// snapshot has been saved, saving another overwrites it in place and
    /// needs no new disk space (#48, ADR-0013); only the first may.
    ///
    /// # Errors
    ///
    /// [`StoreError::Database`] if it can't be written, for instance on a
    /// full disk or a read-only file. [`StoreError::SnapshotTooLarge`] if
    /// its JSON is longer than [`MAX_SNAPSHOT_BYTES`], which no snapshot the
    /// engine writes is.
    pub fn save_snapshot(&mut self, state: &EngineState) -> Result<(), StoreError> {
        let mut json = encode(state, "engine snapshot")?;
        let padding =
            MAX_SNAPSHOT_BYTES
                .checked_sub(json.len())
                .ok_or(StoreError::SnapshotTooLarge {
                    len: u64::try_from(json.len()).unwrap_or(u64::MAX),
                    max: MAX_SNAPSHOT_BYTES,
                })?;
        // Trailing spaces are valid JSON whitespace, so the padded text
        // still parses as `state`.
        json.extend(std::iter::repeat_n(' ', padding));

        // Each statement is its own atomic transaction. The `UPDATE`
        // overwrites a row of the same length in place; only the very first
        // save inserts it.
        let updated = self
            .connection
            .execute("UPDATE engine_snapshot SET json = ?1 WHERE id = 1", [&json])?;
        if updated == 0 {
            self.connection.execute(
                "INSERT INTO engine_snapshot (id, json) VALUES (1, ?1)",
                [&json],
            )?;
        }
        Ok(())
    }

    /// The stored engine snapshot, or `None` if none was ever saved.
    /// Restoring the engine from it, and checking it, is
    /// `Engine::restore`'s (T-2.7).
    ///
    /// # Errors
    ///
    /// [`StoreError::SnapshotTooLarge`] if the stored value is longer than
    /// [`MAX_SNAPSHOT_BYTES`]; its length is checked in SQL, before the
    /// value is read or parsed. [`StoreError::Corrupt`] if it isn't valid
    /// [`EngineState`] JSON. [`StoreError::Database`] if the file can't be
    /// read.
    pub fn load_snapshot(&self) -> Result<Option<EngineState>, StoreError> {
        let max = i64::try_from(MAX_SNAPSHOT_BYTES).unwrap_or(i64::MAX);
        // `octet_length` reads the value's length from the record's header,
        // without loading the value; the value itself is only returned
        // when it is short enough.
        let stored: Option<(i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT octet_length(json), \
                        CASE WHEN octet_length(json) <= ?1 THEN json END \
                 FROM engine_snapshot WHERE id = 1",
                [max],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((len, json)) = stored else {
            return Ok(None);
        };
        let Some(json) = json else {
            return Err(StoreError::SnapshotTooLarge {
                len: u64::try_from(len).unwrap_or(0),
                max: MAX_SNAPSHOT_BYTES,
            });
        };
        decode(&json, "engine snapshot").map(Some)
    }
}

/// Sets up a new connection. Each setting is the connection's own, so this
/// writes nothing to the file.
///
/// # Errors
///
/// [`StoreError::Database`] if SQLite refuses a setting.
fn configure(connection: &Connection) -> Result<(), StoreError> {
    // Keep the rollback journal between transactions, with its header
    // zeroed, instead of deleting it: a later transaction of the same size
    // reuses its space, so a checkpoint needs no new disk space (#48,
    // ADR-0013). DELETE and TRUNCATE recreate or regrow the journal, and
    // WAL appends to a log, on every write.
    // A database file in WAL mode keeps WAL, without any write; `open`
    // checks the mode in effect once it has read the file.
    let _requested: String =
        connection.pragma_update_and_check(None, "journal_mode", "PERSIST", |row| row.get(0))?;
    // Flush the journal and the database at every commit, so that `Ok`
    // means on disk, also after a power cut.
    connection.pragma_update(None, "synchronous", "FULL")?;
    // Statement journals and other temporary data stay in memory, not in
    // files that would need disk space.
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    // Until T-5.3 any user can replace the file, so don't trust SQL inside
    // it: functions with side effects can't run from its triggers or views,
    // and the schema can't be edited through SQL.
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    Ok(())
}

/// The bounds of `[from, to)` in stored milliseconds.
fn range(from: SystemTime, to: SystemTime) -> [i64; 2] {
    [
        wall_time::to_millis_saturating(from),
        wall_time::to_millis_saturating(to),
    ]
}

/// `value` as JSON.
///
/// # Errors
///
/// [`StoreError::Encode`] if serde fails, which never happens for the types
/// the store keeps.
fn encode<T: Serialize + ?Sized>(value: &T, what: &'static str) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::Encode { what })
}

/// The `T` whose JSON is `json`.
///
/// # Errors
///
/// [`StoreError::Corrupt`] naming `what`; serde's message is dropped, as it
/// may quote the stored value.
fn decode<T: DeserializeOwned>(json: &str, what: &'static str) -> Result<T, StoreError> {
    serde_json::from_str(json).map_err(|_| StoreError::Corrupt { what })
}

/// The blocklist in a row of `SELECT id, json FROM blocklist`, checked.
///
/// # Errors
///
/// [`StoreError::Corrupt`] if the JSON can't be decoded or its id isn't the
/// row's; [`StoreError::InvalidStoredBlocklist`] if it fails
/// [`Blocklist::validate`].
fn decode_blocklist(row: &Row<'_>) -> Result<Blocklist, StoreError> {
    let id: String = row.get(0)?;
    let json: String = row.get(1)?;
    let blocklist: Blocklist = decode(&json, "blocklist")?;
    blocklist
        .validate()
        .map_err(|source| StoreError::InvalidStoredBlocklist { source })?;
    if blocklist.id != id {
        return Err(StoreError::Corrupt { what: "blocklist" });
    }
    Ok(blocklist)
}

/// The record in a row of `SELECT started_at_ms, blocklist_id, preset,
/// lock, outcome, focus_ns FROM session_history`, checked.
///
/// # Errors
///
/// [`StoreError::Corrupt`] if a value can't be decoded or is out of range.
fn decode_session(row: &Row<'_>) -> Result<SessionRecord, StoreError> {
    const WHAT: &str = "session record";
    let corrupt = || StoreError::Corrupt { what: WHAT };
    let started_at = wall_time::from_millis(row.get(0)?).ok_or_else(corrupt)?;
    let preset: PomodoroPreset = decode(&row.get::<_, String>(2)?, WHAT)?;
    let lock: LockKind = decode(&row.get::<_, String>(3)?, WHAT)?;
    lock.validate().map_err(|_| corrupt())?;
    let outcome = SessionOutcome::parse(&row.get::<_, String>(4)?).ok_or_else(corrupt)?;
    let focus_ns = u64::try_from(row.get::<_, i64>(5)?).map_err(|_| corrupt())?;
    Ok(SessionRecord {
        started_at,
        blocklist_id: row.get(1)?,
        preset,
        lock,
        outcome,
        focus: Duration::from_nanos(focus_ns),
    })
}

/// The event in a row of `SELECT at_ms, kind, target FROM block_event`.
///
/// # Errors
///
/// [`StoreError::Corrupt`] if a value can't be decoded or is out of range.
fn decode_block_event(row: &Row<'_>) -> Result<BlockEvent, StoreError> {
    let corrupt = || StoreError::Corrupt {
        what: "block event",
    };
    Ok(BlockEvent {
        at: wall_time::from_millis(row.get(0)?).ok_or_else(corrupt)?,
        kind: TargetKind::parse(&row.get::<_, String>(1)?).ok_or_else(corrupt)?,
        target: row.get(2)?,
    })
}

#[cfg(test)]
mod tests {
    use keitora_core::engine::Engine;
    use keitora_core::model::{SitePattern, Target};
    use rusqlite::types::Value;

    use super::*;
    use crate::clock::SystemClock;

    /// A store in a fresh folder named after the test, under the system's
    /// temporary folder.
    fn store(test_name: &str) -> Store {
        let dir = std::env::temp_dir()
            .join("keitora-store-unit-tests")
            .join(format!("{test_name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::open(&dir).expect("opens")
    }

    fn blocklist(id: &str) -> Blocklist {
        Blocklist {
            id: id.to_owned(),
            name: "Work".to_owned(),
            color: "#112233".to_owned(),
            targets: vec![Target::Site {
                pattern: SitePattern::new("example.com"),
            }],
            exceptions: Vec::new(),
            block_during_breaks: false,
        }
    }

    fn record(lock: LockKind, focus: Duration) -> SessionRecord {
        SessionRecord {
            started_at: SystemTime::UNIX_EPOCH,
            blocklist_id: "work".to_owned(),
            preset: PomodoroPreset::default(),
            lock,
            outcome: SessionOutcome::Completed,
            focus,
        }
    }

    /// The first second after the Unix epoch, as a `[from, to)` range.
    fn first_second() -> (SystemTime, SystemTime) {
        let from = SystemTime::UNIX_EPOCH;
        (
            from,
            from.checked_add(Duration::from_secs(1)).expect("in range"),
        )
    }

    #[test]
    fn the_connection_keeps_its_journal_and_flushes_every_commit() {
        let store = store("pragmas");
        let pragma = |name: &str| -> String {
            match store
                .connection
                .pragma_query_value(None, name, |row| row.get::<_, Value>(0))
                .expect("reads")
            {
                Value::Text(text) => text,
                Value::Integer(number) => number.to_string(),
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(pragma("journal_mode"), "persist");
        // 2 is FULL.
        assert_eq!(pragma("synchronous"), "2");
        // 2 is MEMORY.
        assert_eq!(pragma("temp_store"), "2");
        assert_eq!(pragma("trusted_schema"), "0");
        assert_eq!(pragma("user_version"), schema::SCHEMA_VERSION.to_string());
    }

    #[test]
    fn every_saved_snapshot_takes_exactly_the_fixed_length() {
        let mut store = store("padding");
        let clock = SystemClock::new();
        let mut engine = Engine::new();
        let idle = engine.snapshot(&clock);
        engine
            .start(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock)
            .expect("starts");
        let strict = engine.snapshot(&clock);

        for state in [&idle, &strict, &idle] {
            store.save_snapshot(state).expect("saves");
            let len: i64 = store
                .connection
                .query_row(
                    "SELECT octet_length(json) FROM engine_snapshot",
                    [],
                    |row| row.get(0),
                )
                .expect("reads");
            assert_eq!(len, i64::try_from(MAX_SNAPSHOT_BYTES).expect("small"));
            assert_eq!(store.load_snapshot().expect("loads").as_ref(), Some(state));
        }
        let rows: i64 = store
            .connection
            .query_row("SELECT count(*) FROM engine_snapshot", [], |row| row.get(0))
            .expect("counts");
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_blocklist_stored_under_another_id_is_corrupt() {
        let mut store = store("id_mismatch");
        store.put_blocklist(&blocklist("work")).expect("stores");
        store
            .connection
            .execute("UPDATE blocklist SET id = 'play'", [])
            .expect("tampers");

        assert!(matches!(
            store.blocklist("play"),
            Err(StoreError::Corrupt { what: "blocklist" })
        ));
        assert!(matches!(
            store.blocklists(),
            Err(StoreError::Corrupt { what: "blocklist" })
        ));
    }

    #[test]
    fn a_stored_blocklist_that_isnt_json_is_corrupt() {
        let mut store = store("blocklist_not_json");
        store.put_blocklist(&blocklist("work")).expect("stores");
        store
            .connection
            .execute(r#"UPDATE blocklist SET json = '{"id":'"#, [])
            .expect("tampers");
        assert!(matches!(
            store.blocklist("work"),
            Err(StoreError::Corrupt { what: "blocklist" })
        ));
    }

    #[test]
    fn a_session_keeps_its_focus_time_to_the_nanosecond() {
        let mut store = store("focus_nanos");
        let exact = record(LockKind::None, Duration::new(1_500, 123_456_789));
        store.record_session(&exact).expect("records");
        let (from, to) = first_second();
        assert_eq!(store.history(from, to).expect("reads"), vec![exact]);
    }

    #[test]
    fn a_focus_time_beyond_the_database_is_refused_and_not_stored() {
        let mut store = store("focus_out_of_range");
        let result = store.record_session(&record(LockKind::None, Duration::MAX));
        assert!(matches!(
            result,
            Err(StoreError::OutOfRange { what: "focus time" })
        ));
        let (from, to) = first_second();
        assert_eq!(store.history(from, to).expect("reads"), Vec::new());
    }

    #[test]
    fn a_stored_session_with_an_invalid_lock_is_corrupt() {
        let mut store = store("history_lock");
        store
            .record_session(&record(LockKind::RandomText { len: 8 }, Duration::ZERO))
            .expect("records");
        store
            .connection
            .execute(
                r#"UPDATE session_history SET lock = '{"kind":"random_text","len":0}'"#,
                [],
            )
            .expect("tampers");
        let (from, to) = first_second();
        assert!(matches!(
            store.history(from, to),
            Err(StoreError::Corrupt {
                what: "session record"
            })
        ));
    }

    #[test]
    fn the_tables_refuse_unknown_outcomes_and_kinds() {
        let store = store("checks");
        let outcome = store.connection.execute(
            "INSERT INTO session_history \
               (started_at_ms, blocklist_id, preset, lock, outcome, focus_ns) \
             VALUES (0, 'a', '{}', '{}', 'abandoned', 0)",
            [],
        );
        assert!(outcome.is_err());
        let kind = store.connection.execute(
            "INSERT INTO block_event (at_ms, kind, target) VALUES (0, 'file', 'x')",
            [],
        );
        assert!(kind.is_err());
    }

    #[test]
    fn errors_name_what_failed_never_the_value() {
        let corrupt = StoreError::Corrupt { what: "settings" };
        assert_eq!(corrupt.to_string(), "the stored settings can't be decoded");
        let too_large = StoreError::SnapshotTooLarge {
            len: 4096,
            max: MAX_SNAPSHOT_BYTES,
        };
        assert_eq!(
            too_large.to_string(),
            "the engine snapshot is 4096 bytes long, more than the maximum of 2048"
        );
    }

    #[test]
    fn outcomes_and_kinds_read_back_as_written() {
        for outcome in [SessionOutcome::Completed, SessionOutcome::Stopped] {
            assert_eq!(SessionOutcome::parse(outcome.as_str()), Some(outcome));
        }
        for kind in [TargetKind::Site, TargetKind::App] {
            assert_eq!(TargetKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SessionOutcome::parse("Completed"), None);
        assert_eq!(TargetKind::parse(""), None);
    }
}
