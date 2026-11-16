# 0011 — Persist the engine as a validated snapshot of its active time

- **Status:** Accepted
- **Date:** 2026-11-10

## Context

The service must restore a running session, strict ones included, after a restart or a reboot (T-1.7, T-2.7). A `MonotonicInstant` means nothing across restarts, the wall clock can be changed by the user or in the firmware ([#19](https://github.com/CornelioDev/keitora/issues/19)), and the stored data is input the service must not trust blindly.

## Decision

- `keitora_core::engine::EngineState` is the persisted form of the engine:

  ```json
  { "session": null | {
      "preset": { "focus_min": 25, "short_break_min": 5, "long_break_min": 15, "cycles": 4 },
      "lock": { "kind": "none" | "until_timer_ends" | "random_text", "len": 12 },
      "first_cycle": 1,
      "active": { "secs": 600, "nanos": 0 },
      "paused": false } }
  ```

  `len` appears only with `random_text`. This JSON is a contract: later versions may only add optional fields, and no field of a session ever gets a default. Unknown fields are skipped.
- It stores active time ([ADR-0010](0010-session-time-as-active-time.md)), never a monotonic instant or a wall time. Restoring continues from exactly that active time; time between the snapshot and the restore is never credited, whatever the lock, until #19 decides otherwise.
- `Engine::restore` is the only way from stored data to an engine; `Engine` doesn't implement `Deserialize`. It checks the lock with `LockKind::validate`, the first cycle (1 to `cycles`, and 1 for a strict session, which can't skip a break), and rejects a paused strict session and a session that has already ended. A `PomodoroPreset` deserializes only through `PomodoroPreset::new` ([#15](https://github.com/CornelioDev/keitora/issues/15)).
- Tagged types such as `LockKind` are read through a flat struct, not `#[serde(tag)]`, so unknown fields aren't buffered.

## Alternatives considered

- **Derive `Deserialize` on `Engine`** — a second, unchecked way in, which could produce a paused strict session.
- **Store wall-clock start and end times, and credit the time the machine was off** — changing the date in the firmware would end a strict session.
- **Restore an ended session as idle** — the same effect, but it hides tampering or damage; rejecting it lets the service log it.

## Consequences

- A reboot extends a strict session by the time the machine was off, until #19 decides otherwise.
- Changing the format in a way old files can't follow needs a migration or an optional version field.
- The integrity of a snapshot rests on who can write it: nothing in it can tell an edited active time or lock from a real one, so only `SYSTEM` may write the store (T-5.3).
- The service writes snapshots atomically and keeps the last good one, checkpoints every few seconds, right when a session ends and soon after a restore, and decides what to do with each `RestoreError` (T-2.2, T-2.5, T-2.7).

## Addendum (T-2.5): the snapshot names its blocklist, and when it is saved

- **The snapshot is bound to its blocklist by id ([#49](https://github.com/CornelioDev/keitora/issues/49)).** The service stores the id of the blocklist the session enforces in the snapshot's own value, as an optional top-level field next to `session`:

  ```json
  { "session": { … }, "blocklist_id": "work" }
  ```

  It is written by `Store::save_bound_snapshot`, in the same fixed slot and the same `UPDATE` as the engine state ([ADR-0013](0013-store-snapshot-in-place-and-never-reset.md)), so the two can never disagree or be saved apart. Only the id is stored, never the blocklist's name, targets or exceptions, which stay in their own table and can't change while a strict session runs (#18). `EngineState` skips the field, as it skips every unknown one, so `Store::load_snapshot` reads the engine state exactly as before; `Store::load_bound_snapshot` reads both, and refuses as corrupt an id that isn't a valid blocklist id or comes with no session. The field is written only with a session; an idle snapshot has none. A snapshot without one, written by `Store::save_snapshot` or by an earlier build, reads with no id; what a session without an id means on restore is T-2.7's to decide. The largest bound snapshot (the longest preset, a 256-character text lock, a 64-byte id) is 286 bytes, far below the slot's 2048.
- **When it is saved.** The service engine (`keitora_service::engine`) saves a snapshot before it answers a request that starts, pauses, resumes, skips a break of or stops a session, and during the tick or request that first sees a phase change or the end of a session; the end of a session is saved at once, as the idle snapshot. While a session runs, a tick also saves a checkpoint of its active time once the last one is 15 seconds old (`CHECKPOINT_INTERVAL`), and the service saves one last checkpoint when it stops. After a crash or a power cut a session resumes from its last checkpoint: up to 15 seconds longer, never shorter. A paused session's snapshot doesn't change, so it gets no periodic checkpoint.
- **Failures ([#48](https://github.com/CornelioDev/keitora/issues/48)).** A strict session starts only once its first snapshot is on disk (`Ok` from the store); otherwise the start is refused with `internal` and the engine is left as it was. Any later save that fails is logged at `WARN`, keeps the session in memory as it is, strict or not, and is retried by every tick until it succeeds; the store keeps the last snapshot saved meanwhile. On a store with no snapshot, creating the engine saves the idle one, so that the slot's space exists before any strict start; if that fails, the service runs all the same and ticks retry it. A stored snapshot is never overwritten by creating the engine or by an idle engine's checkpoint.
- **A `RandomText` session's text isn't in the snapshot.** Users can read the database until T-5.3 reconsiders it, and a text they could read would add no friction. A restored `RandomText` session (T-2.7) needs a new text.
