// SPDX-License-Identifier: MIT OR Apache-2.0

//! The database schema, as a list of migrations applied in order by
//! `rusqlite_migration`, which records how many ran in SQLite's
//! `PRAGMA user_version`.
//!
//! **A released migration is never edited.** Every change to the schema is
//! a new migration appended to [`MIGRATIONS`]: a database written by an
//! older version is brought up to date by running the ones it lacks, and
//! one written by a newer version is refused (`StoreError::SchemaTooNew`).
//!
//! Every table is `STRICT`, so SQLite refuses a value of the wrong type
//! instead of converting it. Values that keitora-core defines as JSON
//! (blocklists, settings, presets, locks, the engine snapshot) are stored as
//! that JSON text, so that their format has one definition; every other
//! value has its own column.
//!
//! A file is only used if its schema is exactly the one its version should
//! have ([`verify`]): until T-5.3 any user can replace it, and an object
//! planted in it, such as a view or a trigger named like a store table or a
//! virtual table running SQLite's FTS or R-Tree code, could make writes
//! vanish or run code the store doesn't expect.

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

use super::StoreError;

/// The SQL of each migration, oldest first. Migration `n` (from 1) takes the
/// schema from version `n - 1` to version `n`.
const MIGRATIONS: [&str; 1] = [
    // 1 — the first schema (T-2.2).
    "
    -- Blocklists, as the JSON of keitora_core::model::Blocklist. `position`
    -- (the rowid) keeps the order they were first stored in; replacing one
    -- updates `json` in place. `id` repeats the JSON's id, to find a list
    -- by it; ids compare exactly (BINARY), case included.
    CREATE TABLE blocklist (
        position INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        json TEXT NOT NULL
    ) STRICT;

    -- The settings, as the JSON of keitora_service::store::Settings. One
    -- row at most; without one, the defaults apply.
    CREATE TABLE settings (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        json TEXT NOT NULL
    ) STRICT;

    -- Sessions that ended, completed or stopped. Times in milliseconds
    -- since the Unix epoch; `seq` (the rowid) breaks ties in the order
    -- they were recorded.
    CREATE TABLE session_history (
        seq INTEGER PRIMARY KEY,
        started_at_ms INTEGER NOT NULL,
        blocklist_id TEXT NOT NULL,
        preset TEXT NOT NULL,
        lock TEXT NOT NULL,
        outcome TEXT NOT NULL CHECK (outcome IN ('completed', 'stopped')),
        focus_ns INTEGER NOT NULL CHECK (focus_ns >= 0)
    ) STRICT;
    CREATE INDEX session_history_by_time ON session_history (started_at_ms, seq);

    -- Every time a site or an app was blocked: the rule that matched, never
    -- the visited URL.
    CREATE TABLE block_event (
        seq INTEGER PRIMARY KEY,
        at_ms INTEGER NOT NULL,
        kind TEXT NOT NULL CHECK (kind IN ('site', 'app')),
        target TEXT NOT NULL
    ) STRICT;
    CREATE INDEX block_event_by_time ON block_event (at_ms, seq);

    -- The engine snapshot (ADR-0011), one row at most, its JSON padded with
    -- spaces to a fixed length so that each save overwrites the same bytes
    -- (ADR-0013).
    CREATE TABLE engine_snapshot (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        json TEXT NOT NULL
    ) STRICT;
    ",
];

/// The schema version a database has once every migration ran: the number
/// of migrations.
pub(super) const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// The migrations, for `rusqlite_migration`.
pub(super) fn migrations() -> Migrations<'static> {
    Migrations::new(MIGRATIONS.iter().map(|sql| M::up(sql)).collect())
}

/// One row of `sqlite_schema`: the object's type, name, table and SQL.
type SchemaRow = (String, String, String, Option<String>);

/// Every object in the database's schema, in a fixed order.
///
/// # Errors
///
/// SQLite's error if the schema can't be read.
fn read_schema(connection: &Connection) -> rusqlite::Result<Vec<SchemaRow>> {
    let mut query = connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name")?;
    let rows = query.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    rows.collect()
}

/// Checks that the database's schema is exactly the one a database at
/// `version` has: what the first `version` migrations create in an empty
/// database, object for object, SQL text included. Reads only.
///
/// # Errors
///
/// [`StoreError::UnexpectedSchema`] if it isn't, with the file left as it
/// was. [`StoreError::Database`] if the schema can't be read.
pub(super) fn verify(connection: &Connection, version: i64) -> Result<(), StoreError> {
    let version = usize::try_from(version).map_err(|_| StoreError::UnexpectedSchema)?;
    let mut expected = Connection::open_in_memory()?;
    migrations().to_version(&mut expected, version)?;
    if read_schema(connection)? == read_schema(&expected)? {
        Ok(())
    } else {
        Err(StoreError::UnexpectedSchema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migrations_are_valid_and_reach_the_schema_version() {
        migrations().validate().expect("valid migrations");
        let mut connection = Connection::open_in_memory().expect("opens");
        migrations().to_latest(&mut connection).expect("migrates");
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("reads");
        assert_eq!(version, SCHEMA_VERSION);
        verify(&connection, SCHEMA_VERSION).expect("the schema it creates is the expected one");
    }

    #[test]
    fn an_empty_database_has_the_schema_of_version_0_only() {
        let connection = Connection::open_in_memory().expect("opens");
        verify(&connection, 0).expect("empty is version 0");
        assert!(matches!(
            verify(&connection, SCHEMA_VERSION),
            Err(StoreError::UnexpectedSchema)
        ));
        assert!(matches!(
            verify(&connection, -1),
            Err(StoreError::UnexpectedSchema)
        ));
    }

    #[test]
    fn any_extra_or_changed_object_is_refused() {
        for change in [
            "CREATE TABLE extra (x)",
            "CREATE INDEX extra ON blocklist (json)",
            "CREATE TRIGGER extra AFTER INSERT ON blocklist BEGIN SELECT 1; END",
            "CREATE VIEW extra AS SELECT 1",
            "DROP INDEX block_event_by_time",
            "DROP TABLE settings; CREATE TABLE settings (id INTEGER PRIMARY KEY, json TEXT)",
        ] {
            let mut connection = Connection::open_in_memory().expect("opens");
            migrations().to_latest(&mut connection).expect("migrates");
            connection.execute_batch(change).expect("test setup");
            assert!(
                matches!(
                    verify(&connection, SCHEMA_VERSION),
                    Err(StoreError::UnexpectedSchema)
                ),
                "{change}"
            );
        }
    }
}
