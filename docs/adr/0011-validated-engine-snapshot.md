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
