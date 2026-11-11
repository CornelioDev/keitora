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
//!   parsed ([`MAX_SNAPSHOT_BYTES`]); no other value longer than 8 MiB is
//!   read at all.
//! - The schema must be exactly the one its version should have
//!   ([`StoreError::UnexpectedSchema`]), and SQL stored in the file never
//!   runs: triggers and views are off, and so is WAL, which SQLite would
//!   otherwise take from a `keitora.db-wal` file planted next to the
//!   database.
//! - A file that isn't a database, is damaged, or comes from a newer schema
//!   is refused with an error and **never deleted, reset or downgraded**:
//!   that would also erase a strict session.
//!
//! # A full disk can't erase a strict session (#48, ADR-0013)
//!
//! - Opening an up-to-date database never writes the database file, so a
//!   store whose file can't be written still opens and reads, and the
//!   service can restore a session; every write then fails with
//!   [`StoreError::Database`]. Opening does write the rollback journal, see
//!   below.
//! - [`Store::save_snapshot`] returns `Ok` only once the snapshot is on
//!   disk (`PRAGMA synchronous = FULL`), and is atomic: on error, the
//!   snapshot stored before is still the one stored.
//! - Once a snapshot has been saved, saving another needs no new disk
//!   space: the snapshot is padded to a fixed length and overwrites the same
//!   page, and the rollback journal is kept between writes
//!   (`journal_mode = PERSIST`) at the size those writes need. A crash
//!   during a write leaves a journal that the next open plays back and, as
//!   SQLite does that before it can keep the journal, deletes; opening then
//!   grows it back to that size, which may fail on a full disk.
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
use keitora_core::model::{Blocklist, MAX_ID_LEN};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

mod connection;
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
    /// An up-to-date database file is only read, so this works even when
    /// the file can't be written, as on a full disk. The rollback journal is
    /// written, to keep room for the next checkpoint
    /// (`connection::reserve_journal`); if that fails, opening still
    /// succeeds.
    ///
    /// # Errors
    ///
    /// - [`StoreError::CreateFolder`] if the folder can't be created.
    /// - [`StoreError::SchemaTooNew`] if the file's schema is newer than this
    ///   build's, [`StoreError::Corrupt`] if its schema version is negative,
    ///   and [`StoreError::UnexpectedSchema`] if its schema isn't the one
    ///   its version should have. The file is left as it was.
    /// - [`StoreError::Database`] if the file isn't a database, is damaged,
    ///   or can't be migrated, for instance because it can't be written.
    ///   The file is never deleted or reset.
    /// - [`StoreError::Migration`] if a migration fails for another reason.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let data_dir = data_dir.as_ref();
        fs::create_dir_all(data_dir).map_err(|source| StoreError::CreateFolder { source })?;
        // Absolute, so that SQLite never reads the name as a URI.
        let path = std::path::absolute(data_dir.join(DB_FILE_NAME))
            .map_err(|source| StoreError::CreateFolder { source })?;
        let mut connection = connection::open(&path)?;

        // The first read. It plays back the journal of a transaction a
        // crash interrupted, if any; nothing else before the checks below
        // can write.
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

        schema::verify(&connection, found)?;

        connection::set_durability(&connection)?;
        if found < schema::SCHEMA_VERSION {
            schema::migrations().to_latest(&mut connection)?;
            tracing::info!(
                from = found,
                to = schema::SCHEMA_VERSION,
                "migrated the database schema"
            );
        }
        connection::reserve_journal(&mut connection);
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
        let changed = self.connection.execute(
            "INSERT INTO blocklist (id, json) VALUES (?1, ?2) \
             ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            params![blocklist.id, json],
        )?;
        one_row_changed(changed, "blocklist")
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
        let changed = self.connection.execute(
            "INSERT INTO settings (id, json) VALUES (1, ?1) \
             ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            [json],
        )?;
        one_row_changed(changed, "settings")
    }

    // -----------------------------------------------------------------------
    // Session history
    // -----------------------------------------------------------------------

    /// Adds `record` to the history.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidRecord`] if its blocklist id isn't a valid id
    /// (as [`Blocklist::validate`] checks one) or its lock fails
    /// [`LockKind::validate`]; nothing is stored. Its preset is always
    /// valid, as [`PomodoroPreset`] can only hold valid values.
    /// [`StoreError::OutOfRange`] if its start time or focus time doesn't
    /// fit in the database's integers, which no time Windows can hold and
    /// no session's focus reaches. [`StoreError::Database`] if the file
    /// can't be written.
    pub fn record_session(&mut self, record: &SessionRecord) -> Result<(), StoreError> {
        if !is_valid_blocklist_id(&record.blocklist_id) {
            return Err(StoreError::InvalidRecord {
                what: "blocklist id",
            });
        }
        record
            .lock
            .validate()
            .map_err(|_| StoreError::InvalidRecord { what: "lock" })?;
        let started_at_ms = wall_time::to_millis(record.started_at)
            .ok_or(StoreError::OutOfRange { what: "wall time" })?;
        let focus_ns = i64::try_from(record.focus.as_nanos())
            .map_err(|_| StoreError::OutOfRange { what: "focus time" })?;
        let changed = self.connection.execute(
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
        one_row_changed(changed, "session record")
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
        let changed = self.connection.execute(
            "INSERT INTO block_event (at_ms, kind, target) VALUES (?1, ?2, ?3)",
            params![at_ms, event.kind.as_str(), event.target],
        )?;
        one_row_changed(changed, "block event")
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
        let changed = if updated == 0 {
            self.connection.execute(
                "INSERT INTO engine_snapshot (id, json) VALUES (1, ?1)",
                [&json],
            )?
        } else {
            updated
        };
        one_row_changed(changed, "engine snapshot")
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

/// Checks that a write changed exactly one row. With the store's own
/// schema it always does; anything else means the write may not have been
/// stored.
///
/// # Errors
///
/// [`StoreError::WriteNotApplied`] naming `what` otherwise.
fn one_row_changed(changed: usize, what: &'static str) -> Result<(), StoreError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(StoreError::WriteNotApplied { what })
    }
}

/// Whether `id` is a valid blocklist id, by the rule of
/// [`Blocklist::validate`]: 1 to [`MAX_ID_LEN`] bytes, ASCII letters,
/// digits and `-` only.
fn is_valid_blocklist_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
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
    let blocklist_id: String = row.get(1)?;
    if !is_valid_blocklist_id(&blocklist_id) {
        return Err(corrupt());
    }
    let preset: PomodoroPreset = decode(&row.get::<_, String>(2)?, WHAT)?;
    let lock: LockKind = decode(&row.get::<_, String>(3)?, WHAT)?;
    lock.validate().map_err(|_| corrupt())?;
    let outcome = SessionOutcome::parse(&row.get::<_, String>(4)?).ok_or_else(corrupt)?;
    let focus_ns = u64::try_from(row.get::<_, i64>(5)?).map_err(|_| corrupt())?;
    Ok(SessionRecord {
        started_at,
        blocklist_id,
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
    use keitora_core::model::{
        AppIdentifiers, MAX_APP_ID_LEN, MAX_EXCEPTIONS, MAX_NAME_LEN, MAX_PATTERN_LEN, MAX_TARGETS,
        SitePattern, Target,
    };
    use rusqlite::config::DbConfig;
    use rusqlite::limits::Limit;
    use rusqlite::types::Value;

    use super::*;
    use crate::clock::SystemClock;

    /// A fresh folder named after a test, under the system's temporary
    /// folder, deleted when dropped.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(test_name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join("keitora-store-unit-tests")
                .join(format!("{test_name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("creates the sandbox");
            Self(dir)
        }

        /// A folder `name` inside the sandbox, as a data folder.
        fn dir(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        fn db(&self, name: &str) -> PathBuf {
            self.dir(name).join(DB_FILE_NAME)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            // Never panic in `drop`; a leftover folder is cleared next run.
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A store in the data folder `data` of a fresh sandbox. Bind both, the
    /// sandbox first, so that the store closes before the folder goes.
    fn store(test_name: &str) -> (Sandbox, Store) {
        let sandbox = Sandbox::new(test_name);
        let store = Store::open(sandbox.dir("data")).expect("opens");
        (sandbox, store)
    }

    /// A connection to `path` that bypasses the store, as a tamperer has.
    fn raw(path: &Path) -> Connection {
        Connection::open(path).expect("opens the file directly")
    }

    /// The snapshot of a strict session that has run for `active`.
    fn strict(active_secs: u64) -> EngineState {
        let state = format!(
            r#"{{"session":{{"preset":{{"focus_min":25,"short_break_min":5,"long_break_min":15,"cycles":4}},"lock":{{"kind":"until_timer_ends"}},"first_cycle":1,"active":{{"secs":{active_secs},"nanos":999999999}},"paused":false}}}}"#
        );
        serde_json::from_str(&state).expect("a valid snapshot")
    }

    /// Every file in `dir`, by name, with its size.
    fn files_in(dir: &Path) -> Vec<(String, u64)> {
        let mut files: Vec<(String, u64)> = fs::read_dir(dir)
            .expect("lists")
            .map(|entry| entry.expect("reads an entry"))
            .filter(|entry| entry.metadata().expect("metadata").is_file())
            .map(|entry| {
                let len = entry.metadata().expect("metadata").len();
                (entry.file_name().into_string().expect("UTF-8"), len)
            })
            .collect();
        files.sort();
        files
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
        let (_sandbox, store) = store("pragmas");
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
        assert_eq!(pragma("cell_size_check"), "1");
        assert_eq!(pragma("user_version"), schema::SCHEMA_VERSION.to_string());
    }

    #[test]
    fn the_connection_trusts_nothing_in_the_file() {
        let (_sandbox, store) = store("hardening");
        let config = |option| store.connection.db_config(option).expect("reads");
        assert!(!config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA));
        assert!(config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE));
        assert!(!config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER));
        assert!(!config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW));
        assert_eq!(
            store
                .connection
                .limit(Limit::SQLITE_LIMIT_LENGTH)
                .expect("reads"),
            connection::MAX_VALUE_BYTES
        );
    }

    // -- WAL is compiled out (security review M1) ---------------------------

    #[test]
    fn write_ahead_logging_is_compiled_out() {
        let (_sandbox, store) = store("no_wal");
        let options: Vec<String> = store
            .connection
            .prepare("PRAGMA compile_options")
            .expect("prepares")
            .query_map([], |row| row.get(0))
            .expect("lists")
            .collect::<Result<_, _>>()
            .expect("reads");
        assert!(
            options.iter().any(|option| option == "OMIT_WAL"),
            "LIBSQLITE3_FLAGS from .cargo/config.toml wasn't applied: {options:?}"
        );
        let mode: String = store
            .connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
            .expect("asks");
        assert_eq!(mode, "persist");
    }

    #[test]
    fn a_planted_wal_file_is_never_opened() {
        let sandbox = Sandbox::new("planted_wal");
        let dir = sandbox.dir("data");
        let mut store = Store::open(&dir).expect("opens");
        store.save_snapshot(&strict(60)).expect("saves");
        drop(store);

        // With WAL compiled in, SQLite would open any `-wal` file before a
        // read, create the `-shm` index next to it, and read pages from it.
        let wal = dir.join("keitora.db-wal");
        let planted = vec![0x37_u8; 64 * 1024];
        fs::write(&wal, &planted).expect("plants a WAL file");

        let store = Store::open(&dir).expect("opens");
        assert_eq!(store.load_snapshot().expect("loads"), Some(strict(60)));
        drop(store);
        assert_eq!(fs::read(&wal).expect("reads"), planted);
        assert!(!dir.join("keitora.db-shm").exists());
    }

    #[test]
    fn a_database_in_wal_mode_is_refused() {
        let sandbox = Sandbox::new("wal_header");
        let dir = sandbox.dir("data");
        drop(Store::open(&dir).expect("opens"));
        // Bytes 18 and 19 of the header are the file format's read and write
        // versions; 2 means WAL.
        let mut file = fs::read(sandbox.db("data")).expect("reads");
        for byte in file.get_mut(18..20).expect("a header") {
            *byte = 2;
        }
        fs::write(sandbox.db("data"), &file).expect("writes");

        let result = Store::open(&dir);
        assert!(
            matches!(result, Err(StoreError::Database { .. })),
            "{result:?}"
        );
        drop(result);
        assert_eq!(fs::read(sandbox.db("data")).expect("reads"), file);
    }

    // -- Planted schema objects (security review M3) ------------------------

    /// Opens a fresh store in `data`, closes it, runs `sql` on the file
    /// directly, and checks that opening it again is refused with the file
    /// left byte for byte as it was.
    fn assert_schema_change_is_refused(test_name: &str, sql: &str) {
        let sandbox = Sandbox::new(test_name);
        let mut store = Store::open(sandbox.dir("data")).expect("opens");
        store.save_snapshot(&strict(60)).expect("saves");
        drop(store);
        raw(&sandbox.db("data"))
            .execute_batch(sql)
            .expect("test setup");
        let before = fs::read(sandbox.db("data")).expect("reads");

        let result = Store::open(sandbox.dir("data"));
        assert!(
            matches!(result, Err(StoreError::UnexpectedSchema)),
            "{sql}: {result:?}"
        );
        drop(result);
        assert_eq!(
            fs::read(sandbox.db("data")).expect("reads"),
            before,
            "{sql}"
        );
    }

    #[test]
    fn a_view_planted_in_place_of_a_table_is_refused() {
        // The reproduction of the review: writes to the view vanish.
        assert_schema_change_is_refused(
            "planted_view",
            "DROP TABLE engine_snapshot;
             CREATE TABLE elsewhere (id INTEGER, json TEXT);
             CREATE VIEW engine_snapshot AS SELECT id, json FROM elsewhere;
             CREATE TRIGGER swallow INSTEAD OF UPDATE ON engine_snapshot BEGIN SELECT 1; END;",
        );
    }

    #[test]
    fn a_planted_trigger_or_virtual_table_is_refused() {
        assert_schema_change_is_refused(
            "planted_trigger",
            "CREATE TRIGGER rewrite AFTER UPDATE ON engine_snapshot \
             BEGIN UPDATE engine_snapshot SET json = '{}'; END;",
        );
        assert_schema_change_is_refused(
            "planted_fts",
            "CREATE VIRTUAL TABLE search USING fts5(text);",
        );
    }

    #[test]
    fn a_version_0_file_with_objects_is_refused_not_migrated() {
        let sandbox = Sandbox::new("version_0_objects");
        fs::create_dir_all(sandbox.dir("data")).expect("creates");
        raw(&sandbox.db("data"))
            .execute_batch("CREATE TABLE planted (x);")
            .expect("test setup");
        let before = fs::read(sandbox.db("data")).expect("reads");

        let result = Store::open(sandbox.dir("data"));
        assert!(matches!(result, Err(StoreError::UnexpectedSchema)));
        drop(result);
        assert_eq!(fs::read(sandbox.db("data")).expect("reads"), before);
    }

    #[test]
    fn a_write_that_changes_no_row_or_several_is_an_error() {
        assert!(one_row_changed(1, "x").is_ok());
        for changed in [0, 2] {
            assert!(matches!(
                one_row_changed(changed, "engine snapshot"),
                Err(StoreError::WriteNotApplied {
                    what: "engine snapshot"
                })
            ));
        }
    }

    // -- The journal across a crash (QA F1) ---------------------------------

    #[test]
    fn reopening_writes_the_journal_but_never_the_database_file() {
        let sandbox = Sandbox::new("reopen_no_db_write");
        let mut store = Store::open(sandbox.dir("data")).expect("opens");
        store.save_snapshot(&strict(60)).expect("saves");
        drop(store);
        let before = fs::read(sandbox.db("data")).expect("reads");

        let store = Store::open(sandbox.dir("data")).expect("opens");
        assert_eq!(store.load_snapshot().expect("loads"), Some(strict(60)));
        drop(store);
        assert_eq!(fs::read(sandbox.db("data")).expect("reads"), before);
    }

    #[test]
    fn checkpoints_after_recovering_from_a_crash_need_no_new_disk_space() {
        let sandbox = Sandbox::new("crash_recovery");
        let mut store = Store::open(sandbox.dir("live")).expect("opens");
        store.save_snapshot(&strict(60)).expect("saves");
        drop(store);

        // A checkpoint is under way, its journal written, when the power
        // goes: copy the database and its hot journal as they are then.
        let live = raw(&sandbox.db("live"));
        // With `synchronous = OFF` SQLite writes the journal's header in
        // full at once; otherwise only when it syncs the journal, just
        // before it writes the database, which the copy would have to hit.
        live.execute_batch(
            "PRAGMA journal_mode = PERSIST;
             PRAGMA synchronous = OFF;
             BEGIN IMMEDIATE;
             UPDATE engine_snapshot SET json = upper(json) WHERE id = 1;",
        )
        .expect("starts a checkpoint");
        let crashed = sandbox.dir("crashed");
        fs::create_dir_all(&crashed).expect("creates");
        for name in [DB_FILE_NAME, "keitora.db-journal"] {
            fs::copy(sandbox.dir("live").join(name), crashed.join(name)).expect("copies");
        }
        drop(live);
        let journal = fs::read(crashed.join("keitora.db-journal")).expect("reads");
        assert_ne!(journal.first(), Some(&0), "test setup: the journal is hot");

        // The service starts again: opening plays the journal back, which
        // deletes it, and then grows it back.
        let mut store = Store::open(&crashed).expect("recovers");
        assert_eq!(store.load_snapshot().expect("loads"), Some(strict(60)));

        // From now on SQLite can't create a side file, as on a full disk.
        for name in ["keitora.db-journal", "keitora.db-wal", "keitora.db-shm"] {
            let path = crashed.join(name);
            if !path.exists() {
                fs::create_dir(&path).expect("blocks a side file");
            }
        }
        let before = files_in(&crashed);
        for minute in 2..=20 {
            store
                .save_snapshot(&strict(minute * 61))
                .expect("a checkpoint needs no new disk space");
            assert_eq!(files_in(&crashed), before, "checkpoint {minute}");
        }
        drop(store);
        for name in ["keitora.db-wal", "keitora.db-shm"] {
            let _ = fs::remove_dir(crashed.join(name));
        }
        let store = Store::open(&crashed).expect("opens");
        assert_eq!(store.load_snapshot().expect("loads"), Some(strict(20 * 61)));
    }

    // -- Value sizes (security review L1) -----------------------------------

    /// A valid blocklist at every maximum of `keitora_core::model`, with
    /// as many characters as validation allows that JSON escapes.
    fn worst_case_blocklist() -> Blocklist {
        let text = |prefix: &str, fill: &str, len: usize| -> String {
            let mut text = prefix.to_owned();
            while text.len() < len {
                text.push_str(fill);
            }
            text
        };
        let targets = (0..MAX_TARGETS)
            .map(|i| Target::App {
                name: text(&format!("{i}"), "\"", MAX_NAME_LEN),
                ids: AppIdentifiers {
                    windows_exe: Some(text(&format!("{i}"), "e", MAX_APP_ID_LEN)),
                    windows_path: Some(text(&format!("C:\\{i}"), "\\", MAX_APP_ID_LEN)),
                    android_package: Some(text("a", "\"", MAX_APP_ID_LEN)),
                    ios_token: Some(text("t", "\\", MAX_APP_ID_LEN)),
                },
            })
            .collect();
        let exceptions = (0..MAX_EXCEPTIONS)
            .map(|i| SitePattern::new(text(&format!("e{i}.example.org/"), "q", MAX_PATTERN_LEN)))
            .collect();
        let blocklist = Blocklist {
            id: text("big-", "a", MAX_ID_LEN),
            name: text("n", "\"", MAX_NAME_LEN),
            color: "#abcdef".to_owned(),
            targets,
            exceptions,
            block_during_breaks: true,
        };
        blocklist.validate().expect("test setup: valid");
        blocklist
    }

    #[test]
    fn the_largest_valid_blocklist_fits_under_the_value_limit() {
        let blocklist = worst_case_blocklist();
        let json = serde_json::to_string(&blocklist).expect("encodes");
        assert!(json.len() <= connection::MAX_BLOCKLIST_JSON_BYTES);

        let (_sandbox, mut store) = store("largest_value");
        store.put_blocklist(&blocklist).expect("stores");
        assert_eq!(
            store.blocklist(&blocklist.id).expect("reads"),
            Some(blocklist)
        );
    }

    #[test]
    fn a_stored_value_over_the_limit_is_an_error_not_read() {
        let sandbox = Sandbox::new("value_over_limit");
        let mut store = Store::open(sandbox.dir("data")).expect("opens");
        store.put_blocklist(&blocklist("work")).expect("stores");
        drop(store);
        // 9 000 000 bytes of text, through a connection without the limit.
        raw(&sandbox.db("data"))
            .execute_batch("UPDATE blocklist SET json = hex(zeroblob(4500000));")
            .expect("tampers");

        let store = Store::open(sandbox.dir("data")).expect("opens");
        assert!(matches!(
            store.blocklist("work"),
            Err(StoreError::Database { .. })
        ));
    }

    // -- Session records (QA F3) --------------------------------------------

    #[test]
    fn a_session_record_with_an_invalid_field_is_refused_and_not_stored() {
        let (_sandbox, mut store) = store("invalid_record");
        let mut bad_id = record(LockKind::None, Duration::ZERO);
        bad_id.blocklist_id = "not an id!".to_owned();
        let mut empty_id = record(LockKind::None, Duration::ZERO);
        empty_id.blocklist_id = String::new();
        let mut long_id = record(LockKind::None, Duration::ZERO);
        long_id.blocklist_id = "a".repeat(MAX_ID_LEN + 1);
        for invalid in [bad_id, empty_id, long_id] {
            assert!(matches!(
                store.record_session(&invalid),
                Err(StoreError::InvalidRecord {
                    what: "blocklist id"
                })
            ));
        }
        assert!(matches!(
            store.record_session(&record(LockKind::RandomText { len: 0 }, Duration::ZERO)),
            Err(StoreError::InvalidRecord { what: "lock" })
        ));
        let (from, to) = first_second();
        assert_eq!(store.history(from, to).expect("reads"), Vec::new());
    }

    #[test]
    fn a_stored_session_with_an_invalid_blocklist_id_is_corrupt() {
        let (_sandbox, mut store) = store("history_blocklist_id");
        store
            .record_session(&record(LockKind::None, Duration::ZERO))
            .expect("records");
        store
            .connection
            .execute("UPDATE session_history SET blocklist_id = 'x y'", [])
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
    fn every_saved_snapshot_takes_exactly_the_fixed_length() {
        let (_sandbox, mut store) = store("padding");
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
        let (_sandbox, mut store) = store("id_mismatch");
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
        let (_sandbox, mut store) = store("blocklist_not_json");
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
        let (_sandbox, mut store) = store("focus_nanos");
        let exact = record(LockKind::None, Duration::new(1_500, 123_456_789));
        store.record_session(&exact).expect("records");
        let (from, to) = first_second();
        assert_eq!(store.history(from, to).expect("reads"), vec![exact]);
    }

    #[test]
    fn a_focus_time_beyond_the_database_is_refused_and_not_stored() {
        let (_sandbox, mut store) = store("focus_out_of_range");
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
        let (_sandbox, mut store) = store("history_lock");
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
        let (_sandbox, store) = store("checks");
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
