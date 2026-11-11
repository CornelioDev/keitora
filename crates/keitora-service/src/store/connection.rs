// SPDX-License-Identifier: MIT OR Apache-2.0

//! Opening the SQLite connection and setting it up, in the order
//! [`Store::open`](super::Store::open) calls these functions:
//!
//! 1. [`open`]: opens the file and sets every limit that needs no read of
//!    it: no SQL from the file is trusted, triggers and views are off, and
//!    no value larger than [`MAX_VALUE_BYTES`] is read.
//! 2. The caller reads the schema version and checks the schema
//!    (`schema::verify`).
//! 3. [`set_durability`]: the journal mode and `synchronous`, which SQLite
//!    only sets after reading the schema.
//! 4. The caller applies migrations, then [`reserve_journal`] makes the
//!    rollback journal as large as a checkpoint needs.
//!
//! The bundled SQLite is compiled without write-ahead logging
//! (`SQLITE_OMIT_WAL`, in `.cargo/config.toml`): a `keitora.db-wal` file
//! next to the database is never opened, and a database whose header asks
//! for WAL is refused as not a database (ADR-0013).

use std::path::Path;

use keitora_core::model::{
    MAX_APP_ID_LEN, MAX_EXCEPTIONS, MAX_ID_LEN, MAX_NAME_LEN, MAX_PATTERN_LEN, MAX_TARGETS,
};
use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use super::{StoreError, schema};

/// The most bytes a text of `len` bytes can take in JSON: its two quotes,
/// and every byte escaped as `\"` or `\\` at worst. Validated text has no
/// control characters, the only ones `serde_json` writes longer.
const fn json_text(len: usize) -> usize {
    len.saturating_mul(2).saturating_add(2)
}

/// More than enough for a JSON object's keys and punctuation, around its
/// texts, in a blocklist or one of its targets.
const JSON_OVERHEAD: usize = 128;

/// An upper bound of the JSON of a valid blocklist, about 4.4 MiB: every
/// text at its maximum length and fully escaped, and every target an
/// application with all four identifiers (a site target is shorter).
pub(super) const MAX_BLOCKLIST_JSON_BYTES: usize = JSON_OVERHEAD
    + json_text(MAX_ID_LEN)
    + json_text(MAX_NAME_LEN)
    + json_text("#rrggbb".len())
    + MAX_TARGETS * (JSON_OVERHEAD + json_text(MAX_NAME_LEN) + 4 * json_text(MAX_APP_ID_LEN))
    + MAX_EXCEPTIONS * (1 + json_text(MAX_PATTERN_LEN));

/// The longest text or blob, and the largest row, SQLite reads or writes on
/// the store's connection (`SQLITE_LIMIT_LENGTH`): 8 MiB, almost twice
/// [`MAX_BLOCKLIST_JSON_BYTES`], the largest value the store writes. A
/// larger value in the file, which only tampering can put there, is an
/// error instead of up to 1 GB in memory.
pub(super) const MAX_VALUE_BYTES: i32 = 8 * 1024 * 1024;

// Checked when compiling: raising a maximum in keitora-core so far that a
// valid blocklist wouldn't fit fails the build, instead of failing writes.
const _: () = assert!(
    MAX_BLOCKLIST_JSON_BYTES < MAX_VALUE_BYTES as usize,
    "a valid blocklist's JSON must fit in MAX_VALUE_BYTES"
);

/// Opens the database file at `path`, creating it empty if it doesn't
/// exist, and sets up the connection. Reads nothing from the file.
///
/// `path` must be absolute: this build of SQLite (`SQLITE_USE_URI`) reads a
/// file name that starts with `file:` as a URI with options, which an
/// absolute Windows path never does.
///
/// # Errors
///
/// [`StoreError::Database`] if the file can't be opened or created.
pub(super) fn open(path: &Path) -> Result<Connection, StoreError> {
    // If the file can't be written, SQLite opens it for reading only.
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // Until T-5.3 any user can replace the file, so nothing in it is
    // trusted, set before the first read parses its schema: SQL in the
    // schema can't call functions with side effects, the schema can't be
    // edited through SQL, and triggers and views never run (the store has
    // none, and `schema::verify` refuses any).
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false)?;
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_VALUE_BYTES)?;
    // Check each page's cells as it is read, so that a damaged page is an
    // error rather than a read out of bounds of the page.
    connection.pragma_update(None, "cell_size_check", true)?;
    // Statement journals and other temporary data stay in memory, not in
    // files that would need disk space.
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(connection)
}

/// Keeps the rollback journal between transactions and flushes every
/// commit. SQLite reads the schema before it sets either, so the caller
/// calls this once it has checked the schema; neither writes to the file.
///
/// - `journal_mode = PERSIST`: after each commit SQLite zeroes the journal's
///   header instead of deleting the file, so a later transaction that
///   changes as many pages reuses its space, and a checkpoint needs no new
///   disk space (#48, ADR-0013). DELETE and TRUNCATE recreate or regrow the
///   journal on every write; WAL isn't compiled in.
/// - `synchronous = FULL`: `Ok` from a write means it is on disk, also
///   after a power cut.
///
/// # Errors
///
/// [`StoreError::Database`] if SQLite refuses a setting.
pub(super) fn set_durability(connection: &Connection) -> Result<(), StoreError> {
    // Without WAL, the result is always the mode asked for.
    let _mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "PERSIST", |row| row.get(0))?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

/// Makes the rollback journal as large as a checkpoint needs, so that the
/// next [`Store::save_snapshot`](super::Store::save_snapshot) needs no new
/// disk space, even if opening just deleted the journal.
///
/// SQLite can only set PERSIST after reading the file, and that read rolls
/// back a transaction a crash interrupted, in the default DELETE mode,
/// which deletes the journal. So this journals the two pages a checkpoint
/// changes, page 1 (its change counter) and the snapshot's page, then rolls
/// the transaction back: the journal grows to the size a checkpoint needs,
/// and the database file isn't written, since the changed pages never left
/// SQLite's cache. Without a snapshot it journals page 1 only, and the first
/// save takes the space.
///
/// Best effort: on a read-only file or a full disk it fails, which is
/// logged, and the store opens all the same, so that the service can
/// restore a session; a checkpoint may then fail until there is space.
pub(super) fn reserve_journal(connection: &mut Connection) {
    if let Err(error) = journal_checkpoint_pages(connection) {
        // SQLite's message names no stored value; the schema is the store's
        // own, checked by `schema::verify`.
        tracing::warn!("couldn't reserve journal space for checkpoints: {error}");
    }
}

/// Journals the pages a checkpoint changes, in a transaction it rolls back.
///
/// # Errors
///
/// SQLite's error if the transaction can't start or a page can't be
/// journaled.
fn journal_checkpoint_pages(connection: &mut Connection) -> rusqlite::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Writing the header journals page 1, even with the same value.
    transaction.pragma_update(None, "user_version", schema::SCHEMA_VERSION)?;
    // SQLite doesn't journal a page whose bytes don't change, so the
    // snapshot is changed; `upper` keeps its length, so it stays in place.
    transaction.execute(
        "UPDATE engine_snapshot SET json = upper(json) WHERE id = 1",
        [],
    )?;
    transaction.rollback()
}
