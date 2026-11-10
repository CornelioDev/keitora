// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-1.7: a serializable `EngineState` snapshot,
//! restorable after a reboot with the correct remaining time.
//!
//! # Expected public API
//!
//! Everything lives in `keitora_core::engine`:
//!
//! - `EngineState`: the snapshot. `Debug`, `Clone`, `PartialEq`,
//!   `serde::Serialize` and `serde::Deserialize`, in the JSON format below.
//! - `Engine::snapshot(&self, &clock) -> EngineState`, with the clock taken
//!   as `&C`, `C: Clock + ?Sized`, like every other engine method. It takes
//!   `&self` and never changes the engine, so the service can call it every
//!   few seconds as a checkpoint, not only at transitions. It records the
//!   active time *at the instant it is called*.
//! - `Engine::restore(state: EngineState, &clock) -> Result<Engine,
//!   RestoreError>`, with the clock as above: the only way to turn stored
//!   data into an engine with a session. The restored session continues on
//!   the given clock's monotonic reading from exactly the active time stored.
//! - `RestoreError`: `Debug` and `std::error::Error` with a non-empty
//!   message. Its variants are the developer's choice.
//! - `Engine` does **not** implement `serde::Deserialize`. If
//!   `PomodoroPreset` implements it, it rejects out-of-range values exactly
//!   like `PomodoroPreset::new`.
//!
//! Invalid stored data may be rejected either when deserializing an
//! `EngineState` or by `Engine::restore`; the tests only require that it is
//! rejected by one of them and that valid data passes both.
//!
//! # JSON format (chosen here)
//!
//! A snapshot is written to disk and must still restore after the service is
//! updated, so its format is part of the contract. An engine with a session:
//!
//! ```json
//! {
//!   "session": {
//!     "preset": { "focus_min": 25, "short_break_min": 5, "long_break_min": 15, "cycles": 4 },
//!     "lock": { "kind": "until_timer_ends" },
//!     "first_cycle": 1,
//!     "active": { "secs": 600, "nanos": 0 },
//!     "paused": false
//!   }
//! }
//! ```
//!
//! - `preset`: the four values of `PomodoroPreset::new`, in whole minutes.
//! - `lock`: tagged by `kind`: `{ "kind": "none" }`,
//!   `{ "kind": "until_timer_ends" }` or `{ "kind": "random_text", "len": 8 }`.
//! - `first_cycle`: the cycle whose focus began when the active time was zero
//!   (1, or the next cycle after a short break was skipped).
//! - `active`: the active time since then (time run, not paused), as serde
//!   writes a `std::time::Duration`; exact to the nanosecond.
//! - `paused`: whether the session is paused.
//!
//! An idle engine (never started, stopped, or its session over) writes
//! `"session": null` (or leaves `session` out); `{ "session": null }` restores
//! as idle. Every field above is required on input: none of them may default,
//! since a missing `lock` must not turn a strict session into an unlocked one.
//! More fields may be written, but only optional ones.
//!
//! # Decisions pinned here (taken by QA as the recommended option, under the
//! maintainer's pre-authorization of 2026-11-10)
//!
//! - **D1** Time between the snapshot and the restore is never counted, for
//!   any session, strict or not: core never credits time from the wall clock
//!   (ADR-0010; #19 stays open for F2/F5). The session resumes with exactly
//!   the remaining time it had when the snapshot was taken.
//! - **D2** `Engine::restore` is a constructor, and `Engine` has no
//!   `Deserialize`: there is one way in, and it validates.
//! - **D3** A strict snapshot with a first cycle other than 1 is rejected: a
//!   strict session can't skip a break, so its first cycle is always 1, and a
//!   larger one would cut cycles from it.
//! - **D4** A snapshot whose active time reaches the end of its session is
//!   either rejected or restored as an idle engine; it never comes back as a
//!   session. Both are accepted, as both leave the engine idle.
//! - **D5** Every field is required (see above), and the lock uses the
//!   `kind` tag of `Target` in T-1.1.
//!
//! # Criteria covered
//!
//! - **AC1** — `EngineState` is a serializable snapshot (JSON round trip):
//!   `snapshot_of_a_new_engine_has_no_session`,
//!   `snapshot_round_trips_through_json_text_in_every_state`,
//!   `snapshot_written_in_the_documented_format_restores`,
//!   `lock_kinds_are_written_with_a_kind_tag`,
//!   `snapshot_and_restore_work_with_a_clock_trait_object`.
//! - **AC2** — restorable after a reboot with the correct remaining time:
//!   `restored_session_has_the_exact_remaining_time_of_the_snapshot`,
//!   `restored_session_counts_down_on_the_new_clock_through_every_phase`,
//!   `restored_paused_session_stays_paused_until_resumed`,
//!   `restored_session_after_a_skipped_break_keeps_its_cycle`,
//!   `snapshot_at_a_phase_boundary_restores_into_the_next_phase`,
//!   `restored_strict_session_keeps_every_strict_rule`,
//!   `restored_strict_session_ends_exactly_when_its_time_runs_out`.
//! - **S1** — never persist a `MonotonicInstant`:
//!   `snapshot_does_not_depend_on_the_monotonic_clock_origin`,
//!   `restored_session_has_the_exact_remaining_time_of_the_snapshot` (restored
//!   on clocks whose monotonic reading is behind and far ahead of the old
//!   one).
//! - **S2** — store the active time, the segment's first cycle, the preset
//!   and the lock: `snapshot_records_preset_lock_first_cycle_and_active_time`,
//!   `snapshot_records_a_pause`,
//!   `snapshot_after_a_skipped_break_records_the_new_first_cycle`.
//! - **S3** — checkpoint the active time every few seconds, not only at
//!   transitions: `snapshot_holds_the_active_time_at_the_instant_it_is_taken`,
//!   `snapshot_leaves_out_time_spent_paused`,
//!   `taking_snapshots_never_changes_the_engine`.
//! - **S4** (#15) — restoring goes through the same validation as
//!   `PomodoroPreset::new`: `restore_rejects_every_preset_value_out_of_range`,
//!   `restore_accepts_presets_at_the_limits`,
//!   `preset_deserialization_if_any_keeps_the_limits`.
//! - **S5** — rejects a first cycle outside 1 to `cycles`:
//!   `restore_rejects_a_first_cycle_outside_one_to_cycles`,
//!   `restore_accepts_every_first_cycle_from_one_to_cycles_when_not_strict`,
//!   `restore_rejects_a_strict_session_with_a_first_cycle_after_one` (D3).
//! - **S6** — rejects a strict session that is paused:
//!   `restore_rejects_a_paused_strict_session`,
//!   `restore_accepts_paused_sessions_that_are_not_strict`.
//! - **S7** — never restores a session that had already ended:
//!   `snapshot_of_a_session_that_has_ended_has_no_session`,
//!   `snapshot_after_stop_or_skipping_the_long_break_has_no_session`,
//!   `restore_never_brings_back_a_session_that_had_ended`.
//! - **S8** (#19) — time while the machine was off is never credited to a
//!   strict session from the wall clock; changing the firmware date doesn't
//!   end it: `time_while_off_is_never_credited_to_a_strict_session`,
//!   `wall_clock_changes_after_restore_do_not_move_a_strict_session`,
//!   `time_while_off_is_not_credited_to_sessions_that_are_not_strict` (D1).
//! - **S9** (T-1.6 review) — restoring validates the lock with
//!   `LockKind::validate`: `restore_rejects_a_random_text_length_out_of_range`,
//!   `restore_accepts_random_text_at_the_limits`,
//!   `restore_rejects_an_unknown_or_malformed_lock`.
//! - **S10** (T-1.6 review) — `Engine` can't be built into a paused strict
//!   session through serde or restore:
//!   `engine_does_not_implement_deserialize`,
//!   `restore_rejects_a_paused_strict_session`,
//!   `restore_rejects_a_snapshot_missing_any_field`,
//!   `restore_rejects_values_of_the_wrong_type`.
//!
//! Not covered here: when and where the service writes snapshots, and
//! checkpointing right at the end of a session (T-2.5, T-2.7); how time while
//! off, asleep or hibernating is treated by the service's clock (#19, F2/F5).

#![cfg(test)]

use std::marker::PhantomData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keitora_core::clock::{Clock, FakeClock};
use keitora_core::engine::{
    Engine, EngineError, EngineState, LockKind, LockReason, MAX_CYCLES, MAX_FOCUS_MIN,
    MAX_LONG_BREAK_MIN, MAX_RANDOM_TEXT_LEN, MAX_SHORT_BREAK_MIN, MIN_RANDOM_TEXT_LEN, Phase,
    PomodoroPreset, RestoreError, Status,
};
use serde_json::{Value, json};

/// 2026-11-03 09:00:00 UTC, in seconds since the Unix epoch.
const START_UNIX_SECS: u64 = 1_793_696_400;

/// One day.
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// One year of 365 days.
const YEAR: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// The whole default session (25 / 5 / 15 minutes, 4 cycles), in minutes.
const SESSION_MIN: u64 = 130;

/// The default preset's values, in the order of `PomodoroPreset::new`.
const DEFAULT: [u32; 4] = [25, 5, 15, 4];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// 2026-11-03 09:00 UTC.
fn start_wall() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(START_UNIX_SECS))
        .expect("the start time fits in a SystemTime")
}

/// A fake clock at the origin of its monotonic timeline, whose wall reading
/// starts at 2026-11-03 09:00 UTC.
fn fake() -> FakeClock {
    FakeClock::new(start_wall())
}

/// The clock of a service that has just started after a reboot: its
/// monotonic reading is back at the origin, its wall reading is `wall`.
fn rebooted(wall: SystemTime) -> FakeClock {
    FakeClock::new(wall)
}

/// The clock of a service that has been up for `uptime`, so its monotonic
/// reading is far from the origin, with its wall reading at `wall`.
fn up_for(uptime: Duration, wall: SystemTime) -> FakeClock {
    let clock = FakeClock::new(wall);
    clock.advance(uptime).expect("the fake clock advances");
    clock.set_wall(wall);
    clock
}

/// `n` minutes.
fn mins(n: u64) -> Duration {
    Duration::from_secs(n.checked_mul(60).expect("minutes fit in a Duration"))
}

/// Lets `by` pass on `clock`.
fn wait(clock: &FakeClock, by: Duration) {
    clock.advance(by).expect("the fake clock advances");
}

/// `wall` moved `by` later.
fn later(wall: SystemTime, by: Duration) -> SystemTime {
    wall.checked_add(by).expect("the wall time fits")
}

/// The default preset.
fn default_preset() -> PomodoroPreset {
    PomodoroPreset::default()
}

/// A preset from its four values, which must be in range.
fn preset(values: [u32; 4]) -> PomodoroPreset {
    let [focus, short_break, long_break, cycles] = values;
    PomodoroPreset::new(focus, short_break, long_break, cycles).expect("the preset is in range")
}

/// An engine running a session started now.
fn started(preset: PomodoroPreset, lock: LockKind, clock: &FakeClock) -> Engine {
    let mut engine = Engine::new();
    engine
        .start(preset, lock, clock)
        .expect("starting from idle succeeds");
    engine
}

/// A snapshot as JSON.
fn to_json(state: &EngineState) -> Value {
    serde_json::to_value(state).expect("a snapshot serializes")
}

/// The engine's snapshot now, as JSON.
fn snapshot_json(engine: &Engine, clock: &FakeClock) -> Value {
    to_json(&engine.snapshot(clock))
}

/// A snapshot written as text, as it would be stored on disk.
fn saved(state: &EngineState) -> String {
    serde_json::to_string(state).expect("a snapshot serializes")
}

/// A snapshot read back from text the engine wrote.
fn loaded(text: &str) -> EngineState {
    serde_json::from_str(text).expect("a snapshot the engine wrote reads back")
}

/// Restores `json`, or `None` if either deserializing it or `Engine::restore`
/// rejects it. A `RestoreError` must have a message.
fn try_restore(json: &Value, clock: &dyn Clock) -> Option<Engine> {
    let state: EngineState = serde_json::from_value(json.clone()).ok()?;
    let result: Result<Engine, RestoreError> = Engine::restore(state, clock);
    match result {
        Ok(engine) => Some(engine),
        Err(error) => {
            let as_std: &dyn std::error::Error = &error;
            assert!(
                !as_std.to_string().is_empty(),
                "a RestoreError has a message: {error:?}"
            );
            None
        }
    }
}

/// Restores `json`, which must be accepted.
fn restore(json: &Value, clock: &dyn Clock) -> Engine {
    try_restore(json, clock).unwrap_or_else(|| panic!("this snapshot must restore: {json}"))
}

/// Checks that `json` is rejected, not restored.
fn assert_rejected(json: &Value, why: &str) {
    let clock = fake();
    assert!(
        try_restore(json, &clock).is_none(),
        "{why}: must be rejected, but restored from {json}"
    );
}

/// Whether a snapshot's JSON has no session.
fn has_no_session(json: &Value) -> bool {
    matches!(json.get("session"), None | Some(Value::Null))
}

/// Checks that `json` doesn't come back as a session: it is either rejected,
/// or restored as an idle engine that allows everything (D4).
fn assert_no_session_comes_back(json: &Value, why: &str) {
    let clock = fake();
    let Some(mut engine) = try_restore(json, &clock) else {
        return;
    };
    assert_eq!(
        engine.status(&clock).phase(),
        Phase::Idle,
        "{why}: restored as a session from {json}"
    );
    assert_eq!(engine.can_stop(&clock), Ok(()), "{why}");
    assert_eq!(engine.can_edit_blocklist(&clock), Ok(()), "{why}");
    assert!(
        has_no_session(&snapshot_json(&engine, &clock)),
        "{why}: its snapshot still has a session"
    );
    engine
        .start(default_preset(), LockKind::None, &clock)
        .unwrap_or_else(|error| panic!("{why}: a new session can't start: {error:?}"));
}

/// The JSON of a lock with no lock.
fn lock_none() -> Value {
    json!({ "kind": "none" })
}

/// The JSON of a strict lock.
fn lock_strict() -> Value {
    json!({ "kind": "until_timer_ends" })
}

/// The JSON of a random-text lock of `len` characters.
fn lock_random_text(len: u32) -> Value {
    json!({ "kind": "random_text", "len": len })
}

/// A snapshot with a session, written by hand in the documented format.
fn session_json(
    preset: [u32; 4],
    lock: Value,
    first_cycle: u32,
    active: Duration,
    paused: bool,
) -> Value {
    let [focus, short_break, long_break, cycles] = preset;
    json!({
        "session": {
            "preset": {
                "focus_min": focus,
                "short_break_min": short_break,
                "long_break_min": long_break,
                "cycles": cycles,
            },
            "lock": lock,
            "first_cycle": first_cycle,
            "active": { "secs": active.as_secs(), "nanos": active.subsec_nanos() },
            "paused": paused,
        }
    })
}

/// `json` with the value at the JSON pointer `path` replaced by `value`.
fn with(json: &Value, path: &str, value: Value) -> Value {
    let mut json = json.clone();
    *json
        .pointer_mut(path)
        .unwrap_or_else(|| panic!("{path} is in the snapshot")) = value;
    json
}

/// `json` without the field at the JSON pointer `path`.
fn without(json: &Value, path: &str) -> Value {
    let mut json = json.clone();
    let (parent, key) = path.rsplit_once('/').expect("a JSON pointer");
    json.pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("{parent} is an object in the snapshot"))
        .remove(key)
        .unwrap_or_else(|| panic!("{path} is in the snapshot"));
    json
}

/// Snapshots `engine`, writes the snapshot as text and restores it on the
/// same clock, then checks that nothing changed.
fn assert_round_trips(engine: &Engine, clock: &FakeClock, what: &str) {
    let state = engine.snapshot(clock);
    let back = loaded(&saved(&state));
    assert_eq!(
        back, state,
        "{what}: the snapshot changed through JSON text"
    );

    let restored = Engine::restore(back, clock).unwrap_or_else(|error| {
        panic!("{what}: a snapshot the engine took must restore: {error:?}")
    });
    assert_eq!(restored.status(clock), engine.status(clock), "{what}");
    assert_eq!(restored.can_stop(clock), engine.can_stop(clock), "{what}");
    assert_eq!(
        restored.can_edit_blocklist(clock),
        engine.can_edit_blocklist(clock),
        "{what}"
    );
    assert_eq!(
        snapshot_json(&restored, clock),
        to_json(&state),
        "{what}: the restored engine's snapshot differs"
    );
}

/// Checks the phase, cycle and remaining time of `status`, and that it isn't
/// paused.
fn assert_running(status: Status, phase: Phase, cycle: u32, remaining: Duration, what: &str) {
    assert_eq!(status.phase(), phase, "{what}: phase");
    assert_eq!(status.cycle(), cycle, "{what}: cycle");
    assert_eq!(status.remaining(), remaining, "{what}: remaining");
    assert!(!status.is_paused(), "{what}: must be running");
}

// Whether a type implements `serde::Deserialize`, checked on stable Rust by
// method resolution: `probe.parse_json(…)` on a `&Probe<T>` finds
// `ViaDeserialize` (no autoref) only if `T: DeserializeOwned`, and otherwise
// falls back to `WithoutDeserialize` (one autoref). Only works on a concrete
// type, hence the macro.

/// A stand-in for `T` in method resolution.
struct Probe<T>(PhantomData<T>);

/// Chosen when `T: DeserializeOwned`.
trait ViaDeserialize {
    /// `Some(true)` if `json` parses as `T`, `Some(false)` if it doesn't.
    fn parse_json(&self, json: &str) -> Option<bool>;
}

impl<T: serde::de::DeserializeOwned> ViaDeserialize for Probe<T> {
    fn parse_json(&self, json: &str) -> Option<bool> {
        Some(serde_json::from_str::<T>(json).is_ok())
    }
}

/// Chosen when `T` doesn't implement `Deserialize`.
trait WithoutDeserialize {
    /// Always `None`: `T` can't be deserialized at all.
    fn parse_json(&self, json: &str) -> Option<bool>;
}

impl<T> WithoutDeserialize for &Probe<T> {
    fn parse_json(&self, _json: &str) -> Option<bool> {
        None
    }
}

/// `None` if `$ty` doesn't implement `Deserialize`; otherwise whether `$json`
/// parses as `$ty`.
macro_rules! parse_if_deserializable {
    ($ty:ty, $json:expr) => {{
        let probe = &Probe::<$ty>(PhantomData);
        probe.parse_json($json)
    }};
}

// ---------------------------------------------------------------------------
// AC1 — a serializable snapshot
// ---------------------------------------------------------------------------

#[test]
fn snapshot_of_a_new_engine_has_no_session() {
    let clock = fake();
    let engine = Engine::new();
    let json = snapshot_json(&engine, &clock);
    assert!(json.is_object(), "a snapshot is a JSON object: {json}");
    assert!(
        has_no_session(&json),
        "an idle engine has no session: {json}"
    );

    let restored = restore(&json, &clock);
    assert_eq!(restored.status(&clock).phase(), Phase::Idle);
    assert_round_trips(&engine, &clock, "idle");
}

#[test]
fn snapshot_round_trips_through_json_text_in_every_state() {
    let clock = fake();

    let engine = started(default_preset(), LockKind::None, &clock);
    assert_round_trips(&engine, &clock, "focus, just started");
    wait(&clock, mins(10) + Duration::from_nanos(123));
    assert_round_trips(&engine, &clock, "focus, mid-phase, odd nanoseconds");

    let clock = fake();
    let engine = started(default_preset(), LockKind::RandomText { len: 8 }, &clock);
    wait(&clock, mins(27));
    assert_round_trips(&engine, &clock, "short break, random text");

    let clock = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(SESSION_MIN - 5));
    assert_round_trips(&engine, &clock, "long break, strict");

    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(40));
    engine
        .pause(&clock)
        .expect("a session that isn't strict pauses");
    wait(&clock, DAY);
    assert_round_trips(&engine, &clock, "paused in the second focus");

    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(57));
    engine
        .skip_break(&clock)
        .expect("a short break can be skipped");
    wait(&clock, mins(3));
    assert_round_trips(&engine, &clock, "after skipping the second short break");

    let clock = fake();
    let longest = preset([
        MAX_FOCUS_MIN,
        MAX_SHORT_BREAK_MIN,
        MAX_LONG_BREAK_MIN,
        MAX_CYCLES,
    ]);
    let engine = started(longest, LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(20 * 60));
    assert_round_trips(&engine, &clock, "longest preset, 20 hours in");

    let clock = fake();
    let engine = started(
        preset([1, 1, 1, 1]),
        LockKind::RandomText {
            len: MAX_RANDOM_TEXT_LEN,
        },
        &clock,
    );
    assert_round_trips(&engine, &clock, "shortest preset, longest random text");
}

#[test]
fn snapshot_written_in_the_documented_format_restores() {
    let clock = fake();

    let idle = restore(&json!({ "session": null }), &clock);
    assert_eq!(idle.status(&clock).phase(), Phase::Idle);

    // 55 minutes into a 50 / 10 / 30 × 3 strict session: the first short
    // break, with 5 minutes left.
    let strict = restore(
        &json!({
            "session": {
                "preset": { "focus_min": 50, "short_break_min": 10, "long_break_min": 30, "cycles": 3 },
                "lock": { "kind": "until_timer_ends" },
                "first_cycle": 1,
                "active": { "secs": 3300, "nanos": 0 },
                "paused": false
            }
        }),
        &clock,
    );
    assert_running(
        strict.status(&clock),
        Phase::ShortBreak,
        1,
        mins(5),
        "documented strict snapshot",
    );
    assert_eq!(strict.can_stop(&clock), Err(LockReason::UntilTimerEnds));

    // Paused 10 minutes and 1 ns into the focus of cycle 2, after the first
    // short break was skipped.
    let paused = restore(
        &json!({
            "session": {
                "preset": { "focus_min": 25, "short_break_min": 5, "long_break_min": 15, "cycles": 4 },
                "lock": { "kind": "random_text", "len": 12 },
                "first_cycle": 2,
                "active": { "secs": 600, "nanos": 1 },
                "paused": true
            }
        }),
        &clock,
    );
    let status = paused.status(&clock);
    assert_eq!(status.phase(), Phase::Focus);
    assert_eq!(status.cycle(), 2);
    assert_eq!(status.remaining(), mins(15) - Duration::from_nanos(1));
    assert!(status.is_paused());
    assert_eq!(paused.can_stop(&clock), Ok(()));
}

#[test]
fn lock_kinds_are_written_with_a_kind_tag() {
    let cases = [
        (LockKind::None, lock_none()),
        (LockKind::UntilTimerEnds, lock_strict()),
        (LockKind::RandomText { len: 12 }, lock_random_text(12)),
    ];
    for (lock, expected) in cases {
        let clock = fake();
        let engine = started(default_preset(), lock, &clock);
        assert_eq!(
            snapshot_json(&engine, &clock).pointer("/session/lock"),
            Some(&expected),
            "{lock:?}"
        );
    }
}

#[test]
fn snapshot_and_restore_work_with_a_clock_trait_object() {
    let fake_clock = fake();
    let clock: &dyn Clock = &fake_clock;
    let mut engine = Engine::new();
    engine
        .start(default_preset(), LockKind::UntilTimerEnds, clock)
        .expect("starts");
    wait(&fake_clock, mins(3));

    let state = engine.snapshot(clock);
    let restored = Engine::restore(state, clock).expect("restores");
    assert_eq!(restored.status(clock), engine.status(clock));
}

// ---------------------------------------------------------------------------
// AC2 — restored after a reboot with the correct remaining time
// ---------------------------------------------------------------------------

#[test]
fn restored_session_has_the_exact_remaining_time_of_the_snapshot() {
    let before = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &before);
    wait(&before, mins(10) + Duration::from_nanos(7));
    let expected = engine.status(&before);
    assert_running(
        expected,
        Phase::Focus,
        1,
        mins(15) - Duration::from_nanos(7),
        "before the reboot",
    );
    let text = saved(&engine.snapshot(&before));

    // A new service has a new monotonic origin: behind the old reading right
    // after a reboot, or far ahead of it after a long uptime.
    let clocks = [
        ("rebooted", rebooted(before.wall())),
        ("up for 1000 days", up_for(DAY * 1000, before.wall())),
        (
            "up for 1 ns",
            up_for(Duration::from_nanos(1), before.wall()),
        ),
    ];
    for (what, clock) in clocks {
        let restored = Engine::restore(loaded(&text), &clock).expect("restores");
        assert_eq!(restored.status(&clock), expected, "{what}");
        assert_eq!(
            restored.can_stop(&clock),
            Err(LockReason::UntilTimerEnds),
            "{what}"
        );
    }
}

#[test]
fn restored_session_counts_down_on_the_new_clock_through_every_phase() {
    let before = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &before);
    wait(&before, mins(40));
    let text = saved(&engine.snapshot(&before));

    let clock = up_for(DAY * 3, before.wall());
    let restored = Engine::restore(loaded(&text), &clock).expect("restores");
    assert_running(
        restored.status(&clock),
        Phase::Focus,
        2,
        mins(15),
        "minute 40",
    );

    // The checkpoint keeps counting from where it was.
    wait(&clock, mins(4));
    assert_eq!(
        snapshot_json(&restored, &clock).pointer("/session/active"),
        Some(&json!({ "secs": 44 * 60, "nanos": 0 })),
        "the active time continues from the snapshot's"
    );

    let steps = [
        (11, Phase::ShortBreak, 2, 5),
        (5, Phase::Focus, 3, 25),
        (25, Phase::ShortBreak, 3, 5),
        (5, Phase::Focus, 4, 25),
        (25, Phase::LongBreak, 4, 15),
    ];
    for (minutes, phase, cycle, remaining) in steps {
        wait(&clock, mins(minutes));
        assert_running(
            restored.status(&clock),
            phase,
            cycle,
            mins(remaining),
            "after the restore",
        );
        assert_eq!(
            restored.can_stop(&clock),
            Err(LockReason::UntilTimerEnds),
            "{phase:?} {cycle}"
        );
    }

    wait(&clock, mins(15) - Duration::from_nanos(1));
    assert_running(
        restored.status(&clock),
        Phase::LongBreak,
        4,
        Duration::from_nanos(1),
        "last nanosecond",
    );
    wait(&clock, Duration::from_nanos(1));
    assert_eq!(restored.status(&clock).phase(), Phase::Idle);
    assert_eq!(restored.can_stop(&clock), Ok(()));
}

#[test]
fn restored_paused_session_stays_paused_until_resumed() {
    let before = fake();
    let mut engine = started(default_preset(), LockKind::None, &before);
    wait(&before, mins(40));
    engine.pause(&before).expect("pauses");
    let paused = engine.status(&before);
    let text = saved(&engine.snapshot(&before));

    let clock = rebooted(later(before.wall(), DAY));
    let mut restored = Engine::restore(loaded(&text), &clock).expect("restores");
    assert_eq!(restored.status(&clock), paused);
    assert!(restored.status(&clock).is_paused());

    wait(&clock, mins(90));
    assert_eq!(restored.status(&clock), paused, "a pause stays frozen");

    restored.resume(&clock).expect("resumes");
    assert_running(
        restored.status(&clock),
        Phase::Focus,
        2,
        mins(15),
        "resumed",
    );
    wait(&clock, mins(15));
    assert_running(
        restored.status(&clock),
        Phase::ShortBreak,
        2,
        mins(5),
        "after resuming",
    );
}

#[test]
fn restored_session_after_a_skipped_break_keeps_its_cycle() {
    let before = fake();
    let mut engine = started(default_preset(), LockKind::None, &before);
    wait(&before, mins(27));
    engine
        .skip_break(&before)
        .expect("skips the first short break");
    wait(&before, mins(5));
    let text = saved(&engine.snapshot(&before));

    let clock = rebooted(before.wall());
    let restored = Engine::restore(loaded(&text), &clock).expect("restores");
    assert_running(
        restored.status(&clock),
        Phase::Focus,
        2,
        mins(20),
        "restored",
    );

    // Left: 20 + 5 of cycle 2, 25 + 5 of cycle 3, 25 + 15 of cycle 4.
    wait(&clock, mins(95) - Duration::from_nanos(1));
    assert_eq!(restored.status(&clock).phase(), Phase::LongBreak);
    wait(&clock, Duration::from_nanos(1));
    assert_eq!(restored.status(&clock).phase(), Phase::Idle);
}

#[test]
fn snapshot_at_a_phase_boundary_restores_into_the_next_phase() {
    let before = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &before);
    wait(&before, mins(25));
    let text = saved(&engine.snapshot(&before));

    let clock = rebooted(before.wall());
    let restored = Engine::restore(loaded(&text), &clock).expect("restores");
    assert_running(
        restored.status(&clock),
        Phase::ShortBreak,
        1,
        mins(5),
        "the focus ended exactly at the snapshot",
    );
}

#[test]
fn restored_strict_session_keeps_every_strict_rule() {
    let before = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &before);
    wait(&before, mins(27));
    let text = saved(&engine.snapshot(&before));

    let clock = rebooted(before.wall());
    let mut restored = Engine::restore(loaded(&text), &clock).expect("restores");
    let status = restored.status(&clock);
    assert_running(status, Phase::ShortBreak, 1, mins(3), "restored");

    assert!(matches!(
        restored.stop(&clock),
        Err(EngineError::StrictSession)
    ));
    assert!(matches!(
        restored.pause(&clock),
        Err(EngineError::StrictSession)
    ));
    assert!(matches!(
        restored.skip_break(&clock),
        Err(EngineError::StrictSession)
    ));
    assert!(
        restored
            .start(preset([1, 1, 1, 1]), LockKind::None, &clock)
            .is_err()
    );
    assert_eq!(restored.can_stop(&clock), Err(LockReason::UntilTimerEnds));
    assert_eq!(
        restored.can_edit_blocklist(&clock),
        Err(LockReason::UntilTimerEnds)
    );
    assert_eq!(restored.status(&clock), status, "refusals change nothing");

    wait(&clock, mins(SESSION_MIN - 27) - Duration::from_nanos(1));
    assert_eq!(restored.status(&clock).phase(), Phase::LongBreak);
    wait(&clock, Duration::from_nanos(1));
    assert_eq!(restored.status(&clock).phase(), Phase::Idle);
    assert_eq!(restored.can_edit_blocklist(&clock), Ok(()));
}

#[test]
fn restored_strict_session_ends_exactly_when_its_time_runs_out() {
    let clock = fake();
    let json = session_json(
        DEFAULT,
        lock_strict(),
        1,
        mins(SESSION_MIN) - Duration::from_nanos(1),
        false,
    );
    let restored = restore(&json, &clock);
    assert_running(
        restored.status(&clock),
        Phase::LongBreak,
        4,
        Duration::from_nanos(1),
        "one nanosecond left",
    );
    assert_eq!(restored.can_stop(&clock), Err(LockReason::UntilTimerEnds));

    wait(&clock, Duration::from_nanos(1));
    assert_eq!(restored.status(&clock).phase(), Phase::Idle);
    assert_eq!(restored.can_stop(&clock), Ok(()));
}

// ---------------------------------------------------------------------------
// S1 — no MonotonicInstant is persisted
// ---------------------------------------------------------------------------

#[test]
fn snapshot_does_not_depend_on_the_monotonic_clock_origin() {
    // Two services in the same state whose monotonic clocks have different
    // origins, with the same wall time, write the same snapshot.
    let fresh = fake();
    let long_up = up_for(DAY * 40 + Duration::from_nanos(777), start_wall());

    let strict_a = started(default_preset(), LockKind::UntilTimerEnds, &fresh);
    let strict_b = started(default_preset(), LockKind::UntilTimerEnds, &long_up);
    let mut free_a = started(default_preset(), LockKind::None, &fresh);
    let mut free_b = started(default_preset(), LockKind::None, &long_up);
    for clock in [&fresh, &long_up] {
        wait(clock, mins(7));
    }
    free_a.pause(&fresh).expect("pauses");
    free_b.pause(&long_up).expect("pauses");
    for clock in [&fresh, &long_up] {
        wait(clock, mins(3));
    }
    free_a.resume(&fresh).expect("resumes");
    free_b.resume(&long_up).expect("resumes");
    for clock in [&fresh, &long_up] {
        wait(clock, mins(4) + Duration::from_nanos(5));
    }

    assert_eq!(
        snapshot_json(&strict_a, &fresh),
        snapshot_json(&strict_b, &long_up),
        "strict"
    );
    assert_eq!(
        snapshot_json(&free_a, &fresh),
        snapshot_json(&free_b, &long_up),
        "paused and resumed"
    );
}

// ---------------------------------------------------------------------------
// S2 — what a snapshot stores
// ---------------------------------------------------------------------------

#[test]
fn snapshot_records_preset_lock_first_cycle_and_active_time() {
    let clock = fake();
    let engine = started(preset([50, 10, 30, 3]), LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(12) + Duration::from_nanos(345));

    let json = snapshot_json(&engine, &clock);
    assert_eq!(
        json.pointer("/session/preset"),
        Some(&json!({
            "focus_min": 50,
            "short_break_min": 10,
            "long_break_min": 30,
            "cycles": 3,
        })),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/lock"),
        Some(&lock_strict()),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/first_cycle"),
        Some(&json!(1)),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/active"),
        Some(&json!({ "secs": 720, "nanos": 345 })),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/paused"),
        Some(&json!(false)),
        "{json}"
    );
}

#[test]
fn snapshot_records_a_pause() {
    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(10));
    engine.pause(&clock).expect("pauses");
    wait(&clock, mins(60));

    let json = snapshot_json(&engine, &clock);
    assert_eq!(
        json.pointer("/session/paused"),
        Some(&json!(true)),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/active"),
        Some(&json!({ "secs": 600, "nanos": 0 })),
        "{json}"
    );
}

#[test]
fn snapshot_after_a_skipped_break_records_the_new_first_cycle() {
    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(27));
    engine.skip_break(&clock).expect("skips");
    let json = snapshot_json(&engine, &clock);
    assert_eq!(
        json.pointer("/session/first_cycle"),
        Some(&json!(2)),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/active"),
        Some(&json!({ "secs": 0, "nanos": 0 })),
        "{json}"
    );

    wait(&clock, mins(5));
    let json = snapshot_json(&engine, &clock);
    assert_eq!(
        json.pointer("/session/first_cycle"),
        Some(&json!(2)),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/active"),
        Some(&json!({ "secs": 300, "nanos": 0 })),
        "{json}"
    );
}

// ---------------------------------------------------------------------------
// S3 — checkpoints
// ---------------------------------------------------------------------------

#[test]
fn snapshot_holds_the_active_time_at_the_instant_it_is_taken() {
    // A checkpoint every 5 seconds, mid-phase, sees the time move each time,
    // not only at transitions.
    let clock = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &clock);
    for step in 1..=6_u64 {
        wait(&clock, Duration::from_secs(5));
        assert_eq!(
            snapshot_json(&engine, &clock).pointer("/session/active"),
            Some(&json!({ "secs": step * 5, "nanos": 0 })),
            "checkpoint {step}"
        );
    }

    // And a checkpoint taken late in the long break restores late in it.
    wait(&clock, mins(SESSION_MIN - 2) - Duration::from_secs(30));
    let text = saved(&engine.snapshot(&clock));
    let after = rebooted(clock.wall());
    let restored = Engine::restore(loaded(&text), &after).expect("restores");
    assert_running(
        restored.status(&after),
        Phase::LongBreak,
        4,
        mins(2),
        "late checkpoint",
    );
}

#[test]
fn snapshot_leaves_out_time_spent_paused() {
    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(10));
    engine.pause(&clock).expect("pauses");
    wait(&clock, mins(60));
    engine.resume(&clock).expect("resumes");
    wait(&clock, mins(5));

    let json = snapshot_json(&engine, &clock);
    assert_eq!(
        json.pointer("/session/active"),
        Some(&json!({ "secs": 900, "nanos": 0 })),
        "{json}"
    );
    assert_eq!(
        json.pointer("/session/paused"),
        Some(&json!(false)),
        "{json}"
    );
}

#[test]
fn taking_snapshots_never_changes_the_engine() {
    let clock = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(26));
    let status = engine.status(&clock);
    let first = engine.snapshot(&clock);
    for _ in 0..100 {
        assert_eq!(engine.snapshot(&clock), first);
    }
    assert_eq!(engine.status(&clock), status);
    wait(&clock, mins(SESSION_MIN - 26) - Duration::from_nanos(1));
    assert_eq!(engine.status(&clock).phase(), Phase::LongBreak);
    wait(&clock, Duration::from_nanos(1));
    assert_eq!(engine.status(&clock).phase(), Phase::Idle, "ended on time");

    // A snapshot doesn't resume a paused session either.
    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(3));
    engine.pause(&clock).expect("pauses");
    let paused = engine.status(&clock);
    let _ = engine.snapshot(&clock);
    wait(&clock, mins(30));
    assert_eq!(engine.status(&clock), paused);
}

// ---------------------------------------------------------------------------
// S4 — the preset is validated like PomodoroPreset::new (#15)
// ---------------------------------------------------------------------------

#[test]
fn restore_rejects_every_preset_value_out_of_range() {
    let base = session_json(DEFAULT, lock_none(), 1, Duration::ZERO, false);
    let fields = [
        ("focus_min", MAX_FOCUS_MIN),
        ("short_break_min", MAX_SHORT_BREAK_MIN),
        ("long_break_min", MAX_LONG_BREAK_MIN),
        ("cycles", MAX_CYCLES),
    ];
    for (field, max) in fields {
        let path = format!("/session/preset/{field}");
        let bad_values = [
            json!(0),
            json!(max + 1),
            json!(u32::MAX),
            json!(u64::from(u32::MAX) + 1),
            json!(-1),
        ];
        for bad in bad_values {
            assert_rejected(
                &with(&base, &path, bad.clone()),
                &format!("{field} = {bad}"),
            );
        }
    }
    // Out of range with a strict lock too.
    let strict = session_json([0, 5, 15, 4], lock_strict(), 1, Duration::ZERO, false);
    assert_rejected(&strict, "strict session with a zero-minute focus");
}

#[test]
fn restore_accepts_presets_at_the_limits() {
    let clock = fake();
    let longest = [
        MAX_FOCUS_MIN,
        MAX_SHORT_BREAK_MIN,
        MAX_LONG_BREAK_MIN,
        MAX_CYCLES,
    ];
    let engine = restore(
        &session_json(longest, lock_strict(), 1, Duration::ZERO, false),
        &clock,
    );
    assert_running(
        engine.status(&clock),
        Phase::Focus,
        1,
        mins(u64::from(MAX_FOCUS_MIN)),
        "longest preset",
    );

    let engine = restore(
        &session_json([1, 1, 1, 1], lock_strict(), 1, Duration::ZERO, false),
        &clock,
    );
    assert_running(
        engine.status(&clock),
        Phase::Focus,
        1,
        mins(1),
        "shortest preset",
    );
}

#[test]
fn preset_deserialization_if_any_keeps_the_limits() {
    // A `PomodoroPreset` is always within its limits; if it can be
    // deserialized on its own, that must not be a way around them.
    let valid = json!({ "focus_min": 25, "short_break_min": 5, "long_break_min": 15, "cycles": 4 });
    let Some(parsed) = parse_if_deserializable!(PomodoroPreset, &valid.to_string()) else {
        return; // Not deserializable: only `PomodoroPreset::new` builds one.
    };
    assert!(parsed, "a valid preset must deserialize: {valid}");

    let fields = [
        ("focus_min", MAX_FOCUS_MIN),
        ("short_break_min", MAX_SHORT_BREAK_MIN),
        ("long_break_min", MAX_LONG_BREAK_MIN),
        ("cycles", MAX_CYCLES),
    ];
    for (field, max) in fields {
        for bad in [0, max + 1, u32::MAX] {
            let json = with(&valid, &format!("/{field}"), json!(bad)).to_string();
            assert_eq!(
                parse_if_deserializable!(PomodoroPreset, &json),
                Some(false),
                "{json} must be rejected"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// S5 — the first cycle is within 1 to `cycles`
// ---------------------------------------------------------------------------

#[test]
fn restore_rejects_a_first_cycle_outside_one_to_cycles() {
    for lock in [lock_none(), lock_random_text(8), lock_strict()] {
        for first_cycle in [0, 5, MAX_CYCLES + 1, u32::MAX] {
            let json = session_json(DEFAULT, lock.clone(), first_cycle, Duration::ZERO, false);
            assert_rejected(&json, &format!("first cycle {first_cycle} of 4, {lock}"));
        }
    }
    let longest = [
        MAX_FOCUS_MIN,
        MAX_SHORT_BREAK_MIN,
        MAX_LONG_BREAK_MIN,
        MAX_CYCLES,
    ];
    let json = session_json(longest, lock_none(), MAX_CYCLES + 1, mins(1), false);
    assert_rejected(&json, "first cycle just after the most cycles");
}

#[test]
fn restore_accepts_every_first_cycle_from_one_to_cycles_when_not_strict() {
    let clock = fake();
    for lock in [lock_none(), lock_random_text(8)] {
        for first_cycle in 1..=4 {
            let engine = restore(
                &session_json(DEFAULT, lock.clone(), first_cycle, Duration::ZERO, false),
                &clock,
            );
            assert_running(
                engine.status(&clock),
                Phase::Focus,
                first_cycle,
                mins(25),
                &format!("first cycle {first_cycle}, {lock}"),
            );
        }
        // From the last cycle, the focus is followed by the long break.
        let engine = restore(&session_json(DEFAULT, lock, 4, mins(25), false), &clock);
        assert_running(
            engine.status(&clock),
            Phase::LongBreak,
            4,
            mins(15),
            "last cycle",
        );
    }

    let longest = [
        MAX_FOCUS_MIN,
        MAX_SHORT_BREAK_MIN,
        MAX_LONG_BREAK_MIN,
        MAX_CYCLES,
    ];
    let engine = restore(
        &session_json(longest, lock_none(), MAX_CYCLES, Duration::ZERO, false),
        &clock,
    );
    assert_eq!(engine.status(&clock).cycle(), MAX_CYCLES);
}

#[test]
fn restore_rejects_a_strict_session_with_a_first_cycle_after_one() {
    // A strict session can't skip a break, so it always counts from cycle 1;
    // a later first cycle would cut cycles from it (D3).
    for first_cycle in 2..=4 {
        let json = session_json(DEFAULT, lock_strict(), first_cycle, mins(1), false);
        assert_rejected(&json, &format!("strict session from cycle {first_cycle}"));
    }
}

// ---------------------------------------------------------------------------
// S6 / S10 — no paused strict session, by any route
// ---------------------------------------------------------------------------

#[test]
fn restore_rejects_a_paused_strict_session() {
    for active in [Duration::ZERO, mins(10), mins(27), mins(SESSION_MIN - 1)] {
        let json = session_json(DEFAULT, lock_strict(), 1, active, true);
        assert_rejected(&json, &format!("strict session paused at {active:?}"));
    }

    // Also when it is made by editing a snapshot the engine wrote.
    let clock = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(10));
    let json = snapshot_json(&engine, &clock);
    assert_rejected(
        &with(&json, "/session/paused", json!(true)),
        "a strict snapshot edited to paused",
    );
}

#[test]
fn restore_accepts_paused_sessions_that_are_not_strict() {
    let clock = fake();
    for lock in [lock_none(), lock_random_text(MIN_RANDOM_TEXT_LEN)] {
        let engine = restore(
            &session_json(DEFAULT, lock.clone(), 1, mins(10), true),
            &clock,
        );
        let status = engine.status(&clock);
        assert_eq!(status.phase(), Phase::Focus, "{lock}");
        assert_eq!(status.remaining(), mins(15), "{lock}");
        assert!(status.is_paused(), "{lock}");
        wait(&clock, mins(20));
        assert_eq!(engine.status(&clock), status, "{lock}: stays frozen");
    }
}

#[test]
fn engine_does_not_implement_deserialize() {
    // The only way from stored data to an engine is `Engine::restore`, which
    // validates (D2); `Engine` must not have a second, unchecked one.
    assert_eq!(
        parse_if_deserializable!(EngineState, r#"{ "session": null }"#),
        Some(true),
        "the probe finds a type that does implement Deserialize"
    );
    let paused_strict = session_json(DEFAULT, lock_strict(), 1, mins(10), true).to_string();
    assert_eq!(
        parse_if_deserializable!(Engine, &paused_strict),
        None,
        "Engine must not implement serde::Deserialize; use Engine::restore"
    );
}

#[test]
fn restore_rejects_a_snapshot_missing_any_field() {
    let strict = session_json(DEFAULT, lock_strict(), 1, mins(10), false);
    let fields = [
        "/session/preset",
        "/session/preset/focus_min",
        "/session/preset/short_break_min",
        "/session/preset/long_break_min",
        "/session/preset/cycles",
        "/session/lock",
        "/session/lock/kind",
        "/session/first_cycle",
        "/session/active",
        "/session/active/secs",
        "/session/paused",
    ];
    for field in fields {
        assert_rejected(&without(&strict, field), &format!("no {field}"));
    }

    let random_text = session_json(DEFAULT, lock_random_text(8), 1, mins(10), false);
    assert_rejected(
        &without(&random_text, "/session/lock/len"),
        "random text with no length",
    );
}

#[test]
fn restore_rejects_values_of_the_wrong_type() {
    let base = session_json(DEFAULT, lock_none(), 1, mins(10), false);
    let cases = [
        ("/session/paused", json!("false")),
        ("/session/paused", json!(0)),
        ("/session/first_cycle", json!(-1)),
        ("/session/first_cycle", json!(1.5)),
        ("/session/first_cycle", json!("1")),
        ("/session/preset/focus_min", json!("25")),
        ("/session/preset/focus_min", json!(25.5)),
        ("/session/active/secs", json!(-600)),
        ("/session/active", json!("600")),
        ("/session/lock", json!(null)),
        ("/session/preset", json!(25)),
        ("/session", json!("strict")),
    ];
    for (path, value) in cases {
        assert_rejected(
            &with(&base, path, value.clone()),
            &format!("{path} = {value}"),
        );
    }
}

// ---------------------------------------------------------------------------
// S7 — an ended session never comes back
// ---------------------------------------------------------------------------

#[test]
fn snapshot_of_a_session_that_has_ended_has_no_session() {
    for lock in [LockKind::UntilTimerEnds, LockKind::None] {
        let clock = fake();
        let engine = started(default_preset(), lock, &clock);
        wait(&clock, mins(SESSION_MIN));
        let json = snapshot_json(&engine, &clock);
        assert!(has_no_session(&json), "{lock:?}: {json}");

        let after = rebooted(clock.wall());
        let restored = restore(&json, &after);
        assert_eq!(restored.status(&after).phase(), Phase::Idle, "{lock:?}");
        assert_eq!(restored.can_stop(&after), Ok(()), "{lock:?}");
    }
}

#[test]
fn snapshot_after_stop_or_skipping_the_long_break_has_no_session() {
    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(10));
    engine.stop(&clock).expect("stops");
    let json = snapshot_json(&engine, &clock);
    assert!(has_no_session(&json), "after stop: {json}");

    let clock = fake();
    let mut engine = started(default_preset(), LockKind::None, &clock);
    wait(&clock, mins(SESSION_MIN - 5));
    engine.skip_break(&clock).expect("skips the long break");
    let json = snapshot_json(&engine, &clock);
    assert!(
        has_no_session(&json),
        "after skipping the long break: {json}"
    );
}

#[test]
fn restore_never_brings_back_a_session_that_had_ended() {
    let max_active = Duration::new(u64::MAX, 999_999_999);
    let cases = [
        (
            session_json(DEFAULT, lock_strict(), 1, mins(SESSION_MIN), false),
            "strict, exactly at its end",
        ),
        (
            session_json(DEFAULT, lock_strict(), 1, mins(SESSION_MIN + 1), false),
            "strict, a minute after its end",
        ),
        (
            session_json(DEFAULT, lock_strict(), 1, YEAR * 10, false),
            "strict, ten years of active time",
        ),
        (
            session_json(DEFAULT, lock_none(), 3, mins(70), false),
            "from cycle 3, exactly at its end",
        ),
        (
            session_json(DEFAULT, lock_none(), 1, mins(SESSION_MIN), true),
            "paused after its end",
        ),
        (
            session_json(DEFAULT, lock_random_text(8), 1, max_active, false),
            "the largest active time",
        ),
        (
            session_json(DEFAULT, lock_strict(), 1, max_active, false),
            "strict, the largest active time",
        ),
    ];
    for (json, why) in cases {
        assert_no_session_comes_back(&json, why);
    }
}

// ---------------------------------------------------------------------------
// S8 — time while off is never credited from the wall clock (#19)
// ---------------------------------------------------------------------------

#[test]
fn time_while_off_is_never_credited_to_a_strict_session() {
    let before = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &before);
    wait(&before, mins(SESSION_MIN - 10));
    let expected = engine.status(&before);
    assert_running(expected, Phase::LongBreak, 4, mins(10), "before");
    let text = saved(&engine.snapshot(&before));

    // Off for a while, or the firmware date moved forwards or backwards.
    let walls = [
        ("1 s later", later(before.wall(), Duration::from_secs(1))),
        ("10 min later", later(before.wall(), mins(10))),
        ("11 min later", later(before.wall(), mins(11))),
        ("a day later", later(before.wall(), DAY)),
        ("ten years later", later(before.wall(), YEAR * 10)),
        ("the Unix epoch", UNIX_EPOCH),
        (
            "a year earlier",
            before.wall().checked_sub(YEAR).expect("fits"),
        ),
    ];
    for (what, wall) in walls {
        let clock = rebooted(wall);
        let restored = Engine::restore(loaded(&text), &clock).expect("restores");
        assert_eq!(restored.status(&clock), expected, "wall clock at {what}");
        assert_eq!(
            restored.can_stop(&clock),
            Err(LockReason::UntilTimerEnds),
            "{what}"
        );
        assert_eq!(
            restored.can_edit_blocklist(&clock),
            Err(LockReason::UntilTimerEnds),
            "{what}"
        );
    }
}

#[test]
fn wall_clock_changes_after_restore_do_not_move_a_strict_session() {
    let before = fake();
    let engine = started(default_preset(), LockKind::UntilTimerEnds, &before);
    wait(&before, mins(SESSION_MIN - 10));
    let text = saved(&engine.snapshot(&before));

    let clock = rebooted(before.wall());
    let restored = Engine::restore(loaded(&text), &clock).expect("restores");
    clock.set_wall(later(before.wall(), YEAR));
    assert_running(
        restored.status(&clock),
        Phase::LongBreak,
        4,
        mins(10),
        "wall a year on",
    );
    clock.set_wall(UNIX_EPOCH);
    wait(&clock, mins(9));
    assert_running(
        restored.status(&clock),
        Phase::LongBreak,
        4,
        mins(1),
        "wall at the epoch, 9 min on",
    );
    assert_eq!(restored.can_stop(&clock), Err(LockReason::UntilTimerEnds));
}

#[test]
fn time_while_off_is_not_credited_to_sessions_that_are_not_strict() {
    // D1: core never credits time from the wall clock, for any lock.
    for lock in [LockKind::None, LockKind::RandomText { len: 8 }] {
        let before = fake();
        let engine = started(default_preset(), lock, &before);
        wait(&before, mins(10));
        let text = saved(&engine.snapshot(&before));

        let clock = rebooted(later(before.wall(), DAY));
        let restored = Engine::restore(loaded(&text), &clock).expect("restores");
        assert_running(
            restored.status(&clock),
            Phase::Focus,
            1,
            mins(15),
            &format!("{lock:?}, a day later"),
        );
    }
}

// ---------------------------------------------------------------------------
// S9 — the lock is validated with LockKind::validate
// ---------------------------------------------------------------------------

#[test]
fn restore_rejects_a_random_text_length_out_of_range() {
    let mut lens = vec![MAX_RANDOM_TEXT_LEN + 1, u32::MAX];
    if let Some(below) = MIN_RANDOM_TEXT_LEN.checked_sub(1) {
        lens.push(below);
    }
    for len in lens {
        for paused in [false, true] {
            let json = session_json(DEFAULT, lock_random_text(len), 1, mins(10), paused);
            assert_rejected(&json, &format!("random text of {len}, paused {paused}"));
        }
    }
}

#[test]
fn restore_accepts_random_text_at_the_limits() {
    let clock = fake();
    for len in [MIN_RANDOM_TEXT_LEN, MAX_RANDOM_TEXT_LEN] {
        let json = session_json(DEFAULT, lock_random_text(len), 1, mins(10), false);
        let engine = restore(&json, &clock);
        assert_eq!(engine.can_stop(&clock), Ok(()), "len {len} isn't strict");
        assert_eq!(
            snapshot_json(&engine, &clock).pointer("/session/lock"),
            Some(&lock_random_text(len)),
            "len {len} is kept"
        );
    }
}

#[test]
fn restore_rejects_an_unknown_or_malformed_lock() {
    let base = session_json(DEFAULT, lock_strict(), 1, mins(10), false);
    let locks = [
        json!({ "kind": "forever" }),
        json!({ "kind": "UntilTimerEnds" }),
        json!({ "kind": "" }),
        json!({ "kind": 1 }),
        json!({ "kind": "random_text", "len": "8" }),
        json!({ "kind": "random_text", "len": -8 }),
        json!({}),
    ];
    for lock in locks {
        assert_rejected(
            &with(&base, "/session/lock", lock.clone()),
            &format!("lock {lock}"),
        );
    }
}

// ---------------------------------------------------------------------------
// The error type
// ---------------------------------------------------------------------------

#[test]
fn restore_error_is_a_standard_error() {
    fn is_std_error<E: std::error::Error + std::fmt::Debug>() {}
    is_std_error::<RestoreError>();
    fn is_snapshot<S: std::fmt::Debug + Clone + PartialEq>() {}
    is_snapshot::<EngineState>();
}
