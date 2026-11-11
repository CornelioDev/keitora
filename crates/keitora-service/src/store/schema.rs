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

use rusqlite_migration::{M, Migrations};

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

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

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
    }
}
