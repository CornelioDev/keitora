# 0013 — Overwrite the engine snapshot in place, and never reset the database

- **Status:** Accepted
- **Date:** 2026-11-11

## Context

The service keeps blocklists, settings, history, block events and the engine snapshot ([ADR-0011](0011-validated-engine-snapshot.md)) in one SQLite file, `%ProgramData%\Keitora\keitora.db` (T-2.2). Two threats shape how it writes that file:

- **A full disk** ([#48](https://github.com/CornelioDev/keitora/issues/48)). A standard user can fill the system volume, as NTFS keeps no space reserved for `SYSTEM`. If a strict session's snapshot can't be written, a reboot restores an older one, often an idle engine, and the strict session is gone. SQLite's default rollback journal (`DELETE`) creates a new journal file on every write, `TRUNCATE` regrows it, and `WAL` appends to a log: all of them need new disk space for every checkpoint.
- **A damaged or foreign file.** Until T-5.3 any user can edit or replace the file, and a newer Keitora may have migrated it. Many apps answer an unreadable or unknown database by recreating it; here that would also erase a strict session.

## Decision

- **The rollback journal is kept** (`PRAGMA journal_mode = PERSIST`): after each commit SQLite zeroes the journal's header instead of deleting the file, so a later transaction that changes as many pages reuses its space. Every commit is flushed (`PRAGMA synchronous = FULL`), so `Store::save_snapshot` returning `Ok` means the snapshot is on disk. Temporary data stays in memory (`PRAGMA temp_store = MEMORY`).
- **The snapshot has a fixed-size slot**: one row of a table of its own, its JSON padded with ASCII spaces to exactly `MAX_SNAPSHOT_BYTES` (2048) bytes, which is valid JSON whitespace. Every save is an `UPDATE` of that row to a value of the same length, which SQLite overwrites in place on the same page, along with page 1 (the file's change counter). The slot fits in one 4 KiB page, so no overflow page is ever involved.
- **Space is allocated by the first save.** The first save `INSERT`s the row into the table's existing root page and journals those same two pages, so after it the database file and the journal are as large as every later checkpoint needs. That first save may need new space and may fail; T-2.5 refuses to start a strict session when it does (#48). Any earlier save, such as an unlocked session's, has already allocated it.
- **The length of a stored snapshot is checked in SQL** (`octet_length`, read from the record's header) before the value is read or parsed; anything longer than `MAX_SNAPSHOT_BYTES` is refused.
- **Opening writes nothing it doesn't have to.** The schema version (`PRAGMA user_version`) is read first; an up-to-date file is only read, so a store on a full disk or a read-only file still opens, and the service can restore a session from it.
- **The store never deletes, resets or downgrades a database.** A file that isn't a database or is damaged is an error, and a newer schema is `StoreError::SchemaTooNew`, with the file left byte for byte as it was. Repairing it is a person's decision.

## Alternatives considered

- **`DELETE`, `TRUNCATE` or `WAL` journal mode** — each needs new disk space on every write (above); the acceptance tests of T-2.2 fail with all three.
- **A separate snapshot file, written to a temporary file and renamed** — the temporary file needs new space on every write, and a second file means a second place to protect and keep consistent.
- **Preallocating the snapshot row when the database is created** — would make the first strict snapshot need no new space either, but a new store must report that it has no snapshot, which needs a marker value the snapshot's JSON format doesn't have.
- **Recreating an unreadable database** — the usual answer for caches, but here it turns damage, or a planted file, into the end of a strict session.

## Consequences

- Checkpoints of a running session need no new disk space; only the first snapshot ever saved, and writes of other kinds (history, block events, blocklists), may. A failed checkpoint keeps the previous snapshot whole (T-2.5 keeps the session in memory and retries).
- The journal file `keitora.db-journal` stays in the data folder, as large as the biggest write so far (at least two pages and a header once a snapshot is saved); it is never truncated.
- A snapshot's JSON can grow to 2048 bytes; the largest the engine writes today is about 250. Growing past that needs a new migration that enlarges the slot.
- A database planted with another page size or in `WAL` mode may lose the guarantee; T-5.3, which stops users from writing the folder, closes that.
- A damaged or newer database can't be opened until someone repairs, restores or removes it; what the service does meanwhile is T-2.5's and T-2.7's to decide. That failure is visible, while silently losing a strict session isn't.
