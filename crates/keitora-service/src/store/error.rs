// SPDX-License-Identifier: MIT OR Apache-2.0

//! [`StoreError`], everything that can go wrong in the store.

use std::io;

use keitora_core::model::ValidationError;

/// Why a [`Store`](super::Store) operation failed.
///
/// No variant holds a stored or given value, such as a blocklist's name or a
/// block event's target, so an error can be logged, printed or formatted
/// with `{:?}` without leaking user data (`docs/architecture.md`,
/// "Storage"). `Corrupt` names what couldn't be decoded, never why: the
/// decoder's message would quote the value.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The data folder, or a folder above it, couldn't be created, or its
    /// absolute path couldn't be found.
    #[error("couldn't create the data folder")]
    CreateFolder {
        /// The file system's error.
        #[source]
        source: io::Error,
    },
    /// SQLite failed: the file isn't a database (a database in WAL mode is
    /// refused as one), is damaged, can't be written (a full disk, a
    /// read-only file), holds a value of the wrong type, or a value longer
    /// than the store reads.
    #[error("database error")]
    Database {
        /// SQLite's error, through `rusqlite`. It never holds a stored value:
        /// every value is a bound parameter, never part of the SQL text. Its
        /// message may name an object of a planted schema (a table or
        /// trigger name) when the schema itself can't be read.
        #[source]
        source: rusqlite::Error,
    },
    /// The database's schema isn't exactly the one its version should have:
    /// an object was added, removed or changed, as by planting a view or a
    /// trigger. The file is left as it was.
    #[error("the database's schema isn't the one its version should have")]
    UnexpectedSchema,
    /// A write didn't change exactly the one row it should have, so it may
    /// not have been stored. Never happens with the store's own schema.
    #[error("the {what} wasn't stored")]
    WriteNotApplied {
        /// What was being written.
        what: &'static str,
    },
    /// [`Store::record_session`](super::Store::record_session) refused a
    /// record with an invalid field; nothing was stored.
    #[error("invalid {what} in the session record")]
    InvalidRecord {
        /// The field: `"blocklist id"` or `"lock"`.
        what: &'static str,
    },
    /// A migration failed for a reason other than an SQLite error.
    #[error("couldn't migrate the database")]
    Migration {
        /// `rusqlite_migration`'s error.
        #[source]
        source: rusqlite_migration::Error,
    },
    /// The database was written by a newer version of Keitora. The store
    /// refuses it and leaves the file as it is: it never downgrades or
    /// resets a database.
    #[error(
        "the database's schema version {found} is newer than {supported}, the newest this version supports"
    )]
    SchemaTooNew {
        /// The file's schema version (`PRAGMA user_version`).
        found: i64,
        /// The newest schema version this build knows.
        supported: i64,
    },
    /// [`Store::put_blocklist`](super::Store::put_blocklist) refused a
    /// blocklist that fails [`Blocklist::validate`]; nothing was stored.
    ///
    /// [`Blocklist::validate`]: keitora_core::model::Blocklist::validate
    #[error("invalid blocklist")]
    InvalidBlocklist(#[source] ValidationError),
    /// A blocklist read from the file fails [`Blocklist::validate`]: the
    /// file was edited or damaged.
    ///
    /// [`Blocklist::validate`]: keitora_core::model::Blocklist::validate
    #[error("a stored blocklist is invalid")]
    InvalidStoredBlocklist {
        /// What `Blocklist::validate` found: the field's path and the
        /// problem, never the value.
        #[source]
        source: ValidationError,
    },
    /// A stored value can't be decoded into its type: the file was edited
    /// or damaged.
    #[error("the stored {what} can't be decoded")]
    Corrupt {
        /// What was being read, such as `"settings"` or `"engine snapshot"`.
        what: &'static str,
    },
    /// The stored engine snapshot is longer than
    /// [`MAX_SNAPSHOT_BYTES`](super::MAX_SNAPSHOT_BYTES), so it wasn't read;
    /// or a snapshot to save would be.
    #[error("the engine snapshot is {len} bytes long, more than the maximum of {max}")]
    SnapshotTooLarge {
        /// Its length, in bytes.
        len: u64,
        /// The maximum, [`MAX_SNAPSHOT_BYTES`](super::MAX_SNAPSHOT_BYTES).
        max: usize,
    },
    /// A value to store doesn't fit in the database's integers. Never
    /// happens for a wall time Windows can hold or a session's focus time.
    #[error("the {what} is out of range")]
    OutOfRange {
        /// What didn't fit, such as `"wall time"`.
        what: &'static str,
    },
    /// A value couldn't be encoded as JSON. Never happens for the types the
    /// store keeps, whose JSON form always exists.
    #[error("couldn't encode the {what}")]
    Encode {
        /// What was being encoded.
        what: &'static str,
    },
}

impl From<rusqlite::Error> for StoreError {
    fn from(source: rusqlite::Error) -> Self {
        Self::Database { source }
    }
}

impl From<rusqlite_migration::Error> for StoreError {
    fn from(error: rusqlite_migration::Error) -> Self {
        match error {
            rusqlite_migration::Error::RusqliteError { err, .. } => Self::Database { source: err },
            source => Self::Migration { source },
        }
    }
}
