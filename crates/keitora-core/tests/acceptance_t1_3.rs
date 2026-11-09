// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-1.3: the pomodoro state machine.
//!
//! # Expected public API
//!
//! Everything lives in `keitora_core::engine`:
//!
//! - `PomodoroPreset`: validated durations and cycles (`Debug`, `Clone`,
//!   `PartialEq`, `Eq`).
//!   - `PomodoroPreset::new(focus_min: u32, short_break_min: u32,
//!     long_break_min: u32, cycles: u32) -> Result<Self, PresetError>`, whole
//!     minutes as in `docs/architecture.md` (`PomodoroPreset { focus_min,
//!     short_break_min, long_break_min, cycles }`). Out-of-range values are
//!     rejected, never clamped.
//!   - `PomodoroPreset::default()`: 25 / 5 / 15 minutes, 4 cycles.
//!   - `focus()`, `short_break()`, `long_break()` -> `Duration`;
//!     `cycles()` -> `u32`.
//! - Limits, as `pub const` (`u32`): `MIN_DURATION_MIN` (1), `MAX_FOCUS_MIN`,
//!   `MAX_SHORT_BREAK_MIN`, `MAX_LONG_BREAK_MIN`, `MIN_CYCLES` (1),
//!   `MAX_CYCLES`.
//! - `PresetError`: implements `std::error::Error`; `field() -> PresetField`
//!   says which value was out of range.
//! - `PresetField`: `Focus`, `ShortBreak`, `LongBreak`, `Cycles` (`Debug`,
//!   `Clone`, `Copy`, `PartialEq`, `Eq`).
//! - `LockKind`: `None` and `UntilTimerEnds` (`#[non_exhaustive]`; T-1.6 adds
//!   `RandomText { len }` and may move the type, re-exporting it here). A
//!   session started with `UntilTimerEnds` is strict.
//! - `Phase`: `Idle`, `Focus`, `ShortBreak`, `LongBreak` (`Debug`, `Clone`,
//!   `Copy`, `PartialEq`, `Eq`).
//! - `Engine`: `Engine::new()` is idle. Every method takes the clock last, as
//!   `&C` with `C: Clock + ?Sized` or as `&dyn Clock`; the tests pass
//!   `&FakeClock` and `&dyn Clock`. Remaining time comes only from
//!   `Clock::monotonic`.
//!   - `start(&mut self, PomodoroPreset, LockKind, &clock) -> Result<(), E>`
//!   - `pause`, `resume`, `skip_break`, `stop`:
//!     `(&mut self, &clock) -> Result<(), E>`
//!   - `status(&self, &clock) -> Status`: observing never changes the
//!     session, so it takes `&self`
//!
//!   `E` is a typed error of the developer's choosing (`Debug`); the tests
//!   only check that misuse is an `Err` and changes nothing.
//! - `Status`: `phase() -> Phase`, `cycle() -> u32` (1-based number of the
//!   focus the phase belongs to: a break keeps the number of the focus before
//!   it), `remaining() -> Duration` (exact, not rounded), `is_paused() ->
//!   bool`. `cycle()` and `remaining()` aren't checked while idle.
//!
//! # Behavior pinned (decided by the maintainer on 2026-11-09)
//!
//! - A session has `cycles` focus phases. Each focus but the last is
//!   followed by a short break, the last by the long break, and the session
//!   ends (back to idle) when the long break ends.
//! - Every phase starts on its own when the previous one runs out; nothing
//!   waits for the user. (A "start breaks automatically" setting is decided
//!   later, in F2/F3, and never applies to strict sessions.)
//! - Only breaks can be skipped. Skipping a short break starts the next focus
//!   with its full duration; skipping the long break ends the session;
//!   skipping a focus is an error. Skipping in a strict session is T-1.6's.
//! - Limits (#15): 1 to 120 min focus, 1 to 30 min short break, 1 to 60 min
//!   long break, 1 to 8 cycles. Out-of-range values are rejected with a
//!   typed error naming the field, never clamped.
//! - The schedule is fixed from the start; only pauses shift it. Observed
//!   late, the engine has moved through every phase that has elapsed, as if
//!   it had been watching all along.
//! - During a break, the cycle number is that of the focus just finished.
//! - A phase ends the instant its remaining time reaches zero: then the next
//!   phase is already current, with its full duration.
//!
//! # Criteria covered
//!
//! - **AC1** — every transition (`Idle → Focus → ShortBreak | LongBreak →
//!   Focus …`, and stop):
//!   `new_engine_is_idle`,
//!   `starting_enters_the_first_focus_with_its_full_duration`,
//!   `remaining_time_counts_down_exactly_on_the_monotonic_clock`,
//!   `focus_moves_to_a_short_break_when_its_time_ends`,
//!   `short_break_moves_to_the_next_focus_when_its_time_ends`,
//!   `default_session_runs_four_focus_phases_then_the_long_break_then_ends`,
//!   `a_new_session_can_start_after_one_ends_and_begins_at_cycle_one`,
//!   `starting_while_a_session_runs_is_rejected_and_changes_nothing`,
//!   `stopping_a_non_strict_session_returns_to_idle_from_every_phase`,
//!   `stopping_when_idle_is_rejected`,
//!   `custom_durations_are_used_for_every_phase`,
//!   `engine_works_with_a_clock_as_a_trait_object`.
//! - **AC2** — pause, non-strict only:
//!   `pausing_freezes_the_phase_and_the_remaining_time`,
//!   `resuming_continues_from_the_frozen_remaining_time`,
//!   `pausing_during_a_break_freezes_the_break`,
//!   `several_pauses_shift_the_schedule_by_their_total_length`,
//!   `pause_and_resume_misuse_is_rejected_and_changes_nothing`,
//!   `stopping_a_paused_session_returns_to_idle`,
//!   `strict_session_refuses_pause_during_focus_and_keeps_running`,
//!   `strict_session_refuses_pause_during_a_break_and_keeps_running`.
//! - **AC3** — skip break:
//!   `skipping_a_short_break_starts_the_next_focus_with_its_full_duration`,
//!   `skipping_the_long_break_ends_the_session`,
//!   `skipping_during_focus_is_rejected_and_changes_nothing`,
//!   `skipping_when_idle_is_rejected`.
//! - **AC4** — long-break boundary:
//!   `only_the_last_focus_of_a_session_is_followed_by_the_long_break`,
//!   `one_cycle_session_goes_from_focus_straight_to_the_long_break`.
//! - **AC5** (#15) — durations and cycles have maximums and minimums, and
//!   out-of-range values are rejected with a typed error:
//!   `default_preset_is_25_5_15_minutes_and_4_cycles`,
//!   `limits_are_pinned`,
//!   `preset_accepts_the_minimum_and_maximum_of_every_value`,
//!   `preset_rejects_zero_for_every_value`,
//!   `preset_rejects_one_above_the_maximum_for_every_value`,
//!   `preset_rejects_the_largest_u32_for_every_value_without_panicking`,
//!   `preset_error_is_a_standard_error_with_a_message`,
//!   `longest_possible_session_runs_to_the_end_without_panicking`.
//! - **AC6** (time, T-1.2 carryover, `docs/threat-model.md` "Change the
//!   system clock") — remaining time comes from the monotonic reading only:
//!   `wall_clock_jumping_a_day_forward_neither_ends_nor_shortens_a_phase`,
//!   `wall_clock_jumping_backwards_neither_extends_nor_resets_a_phase`,
//!   `wall_clock_changes_while_paused_change_nothing`.
//! - **AC7** — late observation (fixed schedule):
//!   `late_observation_advances_through_every_elapsed_phase`,
//!   `late_observation_after_the_session_ended_is_idle`.
//!
//! Not covered here: stop, shortening and edits in strict sessions, and
//! `LockKind::RandomText` (T-1.6); persistence (T-1.7); time while the
//! machine is off (#19, F2/F5).

#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keitora_core::clock::{Clock, FakeClock};
use keitora_core::engine::{
    Engine, LockKind, MAX_CYCLES, MAX_FOCUS_MIN, MAX_LONG_BREAK_MIN, MAX_SHORT_BREAK_MIN,
    MIN_CYCLES, MIN_DURATION_MIN, Phase, PomodoroPreset, PresetField, Status,
};

/// 2026-11-03 09:00:00 UTC, in seconds since the Unix epoch.
const START_UNIX_SECS: u64 = 1_793_696_400;

/// One day, for wall-clock jumps.
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A fake clock whose wall reading starts at 2026-11-03 09:00 UTC.
fn fake() -> FakeClock {
    let wall = UNIX_EPOCH
        .checked_add(Duration::from_secs(START_UNIX_SECS))
        .expect("the start time fits in a SystemTime");
    FakeClock::new(wall)
}

/// `n` minutes.
fn mins(n: u64) -> Duration {
    Duration::from_secs(n.checked_mul(60).expect("minutes fit in a Duration"))
}

/// Lets `by` pass on `clock`.
fn wait(clock: &FakeClock, by: Duration) {
    clock.advance(by).expect("the fake clock advances");
}

/// A validated preset; fails the test if it is rejected.
fn preset(focus: u32, short: u32, long: u32, cycles: u32) -> PomodoroPreset {
    PomodoroPreset::new(focus, short, long, cycles).expect("the preset is in range")
}

/// An engine running a session started now on `clock`.
fn started(preset: PomodoroPreset, lock: LockKind, clock: &FakeClock) -> Engine {
    let mut engine = Engine::new();
    engine
        .start(preset, lock, clock)
        .expect("starting from idle succeeds");
    engine
}

/// Checks a running (or paused) session's phase, cycle and remaining time.
fn assert_running(status: &Status, phase: Phase, cycle: u32, remaining: Duration, paused: bool) {
    assert_eq!(
        (
            status.phase(),
            status.cycle(),
            status.remaining(),
            status.is_paused()
        ),
        (phase, cycle, remaining, paused),
        "(phase, cycle, remaining, paused)"
    );
}

/// Checks that the engine is idle.
fn assert_idle(status: &Status) {
    assert_eq!(status.phase(), Phase::Idle);
    assert!(!status.is_paused(), "an idle engine is never paused");
}

/// The wall reading `by` after `clock`'s current one.
fn wall_plus(clock: &FakeClock, by: Duration) -> SystemTime {
    clock.wall().checked_add(by).expect("the wall time fits")
}

/// The wall reading `by` before `clock`'s current one.
fn wall_minus(clock: &FakeClock, by: Duration) -> SystemTime {
    clock.wall().checked_sub(by).expect("the wall time fits")
}

// ---------------------------------------------------------------------------
// AC1 — transitions
// ---------------------------------------------------------------------------

#[test]
fn new_engine_is_idle() {
    let clock = fake();
    let engine = Engine::new();
    assert_idle(&engine.status(&clock));
}

#[test]
fn starting_enters_the_first_focus_with_its_full_duration() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(25), false);
}

#[test]
fn remaining_time_counts_down_exactly_on_the_monotonic_clock() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);

    wait(&clock, mins(10));
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    wait(&clock, Duration::from_millis(500));
    assert_running(
        &engine.status(&clock),
        Phase::Focus,
        1,
        mins(15) - Duration::from_millis(500),
        false,
    );
}

#[test]
fn focus_moves_to_a_short_break_when_its_time_ends() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);

    wait(&clock, mins(25) - Duration::from_secs(1));
    assert_running(
        &engine.status(&clock),
        Phase::Focus,
        1,
        Duration::from_secs(1),
        false,
    );

    wait(&clock, Duration::from_secs(1));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(5), false);
}

#[test]
fn short_break_moves_to_the_next_focus_when_its_time_ends() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);

    wait(&clock, mins(25) + mins(5) - Duration::from_secs(1));
    assert_running(
        &engine.status(&clock),
        Phase::ShortBreak,
        1,
        Duration::from_secs(1),
        false,
    );

    wait(&clock, Duration::from_secs(1));
    assert_running(&engine.status(&clock), Phase::Focus, 2, mins(25), false);
}

#[test]
fn default_session_runs_four_focus_phases_then_the_long_break_then_ends() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);

    for cycle in 1..=3 {
        assert_running(&engine.status(&clock), Phase::Focus, cycle, mins(25), false);
        wait(&clock, mins(25));
        assert_running(
            &engine.status(&clock),
            Phase::ShortBreak,
            cycle,
            mins(5),
            false,
        );
        wait(&clock, mins(5));
    }
    assert_running(&engine.status(&clock), Phase::Focus, 4, mins(25), false);
    wait(&clock, mins(25));
    assert_running(&engine.status(&clock), Phase::LongBreak, 4, mins(15), false);
    wait(&clock, mins(15) - Duration::from_secs(1));
    assert_running(
        &engine.status(&clock),
        Phase::LongBreak,
        4,
        Duration::from_secs(1),
        false,
    );
    wait(&clock, Duration::from_secs(1));
    assert_idle(&engine.status(&clock));

    wait(&clock, DAY);
    assert_idle(&engine.status(&clock));
}

#[test]
fn a_new_session_can_start_after_one_ends_and_begins_at_cycle_one() {
    let clock = fake();
    let mut engine = started(preset(25, 5, 15, 1), LockKind::None, &clock);
    wait(&clock, mins(25) + mins(15));
    assert_idle(&engine.status(&clock));

    engine
        .start(PomodoroPreset::default(), LockKind::None, &clock)
        .expect("starting after the session ended succeeds");
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(25), false);
}

#[test]
fn starting_while_a_session_runs_is_rejected_and_changes_nothing() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(10));

    assert!(
        engine
            .start(preset(50, 10, 30, 2), LockKind::None, &clock)
            .is_err(),
        "a second start must be rejected"
    );
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    wait(&clock, mins(15));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(5), false);
    assert!(
        engine
            .start(PomodoroPreset::default(), LockKind::None, &clock)
            .is_err()
    );
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(5), false);
}

#[test]
fn stopping_a_non_strict_session_returns_to_idle_from_every_phase() {
    // (time into the session, phase it is in then)
    let points = [
        (Duration::ZERO, Phase::Focus),
        (mins(25), Phase::ShortBreak),
        (mins(30) + mins(7), Phase::Focus),
        (mins(4 * 25 + 3 * 5) + mins(1), Phase::LongBreak),
    ];
    for (offset, phase) in points {
        let clock = fake();
        let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
        wait(&clock, offset);
        assert_eq!(engine.status(&clock).phase(), phase, "at {offset:?}");

        engine
            .stop(&clock)
            .expect("a non-strict session can be stopped");
        assert_idle(&engine.status(&clock));
        wait(&clock, mins(60));
        assert_idle(&engine.status(&clock));
    }
}

#[test]
fn stopping_when_idle_is_rejected() {
    let clock = fake();
    let mut engine = Engine::new();
    assert!(engine.stop(&clock).is_err());
    assert_idle(&engine.status(&clock));
}

#[test]
fn custom_durations_are_used_for_every_phase() {
    let clock = fake();
    let engine = started(preset(50, 10, 30, 2), LockKind::None, &clock);

    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(50), false);
    wait(&clock, mins(50));
    assert_running(
        &engine.status(&clock),
        Phase::ShortBreak,
        1,
        mins(10),
        false,
    );
    wait(&clock, mins(10));
    assert_running(&engine.status(&clock), Phase::Focus, 2, mins(50), false);
    wait(&clock, mins(50));
    assert_running(&engine.status(&clock), Phase::LongBreak, 2, mins(30), false);
    wait(&clock, mins(30));
    assert_idle(&engine.status(&clock));
}

#[test]
fn engine_works_with_a_clock_as_a_trait_object() {
    let fake_clock = fake();
    let clock: &dyn Clock = &fake_clock;
    let mut engine = Engine::new();
    engine
        .start(PomodoroPreset::default(), LockKind::None, clock)
        .expect("start");
    wait(&fake_clock, mins(5));
    engine.pause(clock).expect("pause");
    engine.resume(clock).expect("resume");
    assert_running(&engine.status(clock), Phase::Focus, 1, mins(20), false);
    wait(&fake_clock, mins(20));
    engine.skip_break(clock).expect("skip");
    engine.stop(clock).expect("stop");
    assert_idle(&engine.status(clock));
}

// ---------------------------------------------------------------------------
// AC2 — pause, non-strict only
// ---------------------------------------------------------------------------

#[test]
fn pausing_freezes_the_phase_and_the_remaining_time() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(24));

    engine
        .pause(&clock)
        .expect("a non-strict session can be paused");
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(1), true);

    wait(&clock, mins(60));
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(1), true);
    wait(&clock, DAY);
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(1), true);
}

#[test]
fn resuming_continues_from_the_frozen_remaining_time() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(24));
    engine.pause(&clock).expect("pause");
    wait(&clock, mins(60));

    engine.resume(&clock).expect("resume");
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(1), false);

    wait(&clock, mins(1) - Duration::from_secs(1));
    assert_running(
        &engine.status(&clock),
        Phase::Focus,
        1,
        Duration::from_secs(1),
        false,
    );
    wait(&clock, Duration::from_secs(1));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(5), false);
}

#[test]
fn pausing_during_a_break_freezes_the_break() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(25) + mins(2));

    engine
        .pause(&clock)
        .expect("a non-strict break can be paused");
    wait(&clock, mins(30));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(3), true);

    engine.resume(&clock).expect("resume");
    wait(&clock, mins(3));
    assert_running(&engine.status(&clock), Phase::Focus, 2, mins(25), false);
}

#[test]
fn several_pauses_shift_the_schedule_by_their_total_length() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);

    wait(&clock, mins(5));
    engine.pause(&clock).expect("first pause");
    wait(&clock, mins(7));
    engine.resume(&clock).expect("first resume");
    wait(&clock, mins(5));
    engine.pause(&clock).expect("second pause");
    wait(&clock, mins(11));
    engine.resume(&clock).expect("second resume");

    // 10 minutes of focus have run in 28 minutes.
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);
    wait(&clock, mins(15) + mins(5));
    assert_running(&engine.status(&clock), Phase::Focus, 2, mins(25), false);
}

#[test]
fn pause_and_resume_misuse_is_rejected_and_changes_nothing() {
    let clock = fake();
    let mut idle = Engine::new();
    assert!(idle.pause(&clock).is_err(), "pausing while idle");
    assert!(idle.resume(&clock).is_err(), "resuming while idle");
    assert_idle(&idle.status(&clock));

    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(10));
    assert!(engine.resume(&clock).is_err(), "resuming a running session");
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    engine.pause(&clock).expect("pause");
    wait(&clock, mins(3));
    assert!(engine.pause(&clock).is_err(), "pausing a paused session");
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), true);

    engine.resume(&clock).expect("resume");
    wait(&clock, mins(1));
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(14), false);
}

#[test]
fn stopping_a_paused_session_returns_to_idle() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(10));
    engine.pause(&clock).expect("pause");

    engine.stop(&clock).expect("stop");
    assert_idle(&engine.status(&clock));
    assert!(engine.resume(&clock).is_err(), "nothing left to resume");
}

#[test]
fn strict_session_refuses_pause_during_focus_and_keeps_running() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(10));

    assert!(
        engine.pause(&clock).is_err(),
        "a strict session can't be paused"
    );
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    wait(&clock, mins(15));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(5), false);
}

#[test]
fn strict_session_refuses_pause_during_a_break_and_keeps_running() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(25) + mins(1));

    assert!(
        engine.pause(&clock).is_err(),
        "a strict session can't be paused"
    );
    assert!(engine.resume(&clock).is_err(), "nothing to resume");
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(4), false);

    wait(&clock, mins(4));
    assert_running(&engine.status(&clock), Phase::Focus, 2, mins(25), false);
}

// ---------------------------------------------------------------------------
// AC3 — skip break
// ---------------------------------------------------------------------------

#[test]
fn skipping_a_short_break_starts_the_next_focus_with_its_full_duration() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(25) + mins(2));

    engine
        .skip_break(&clock)
        .expect("a short break can be skipped");
    assert_running(&engine.status(&clock), Phase::Focus, 2, mins(25), false);

    // The schedule continues from the skip.
    wait(&clock, mins(25));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 2, mins(5), false);
}

#[test]
fn skipping_the_long_break_ends_the_session() {
    let clock = fake();
    let mut engine = started(preset(25, 5, 15, 2), LockKind::None, &clock);
    wait(&clock, mins(25 + 5 + 25) + mins(1));
    assert_eq!(engine.status(&clock).phase(), Phase::LongBreak);

    engine
        .skip_break(&clock)
        .expect("the long break can be skipped");
    assert_idle(&engine.status(&clock));
}

#[test]
fn skipping_during_focus_is_rejected_and_changes_nothing() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(10));

    assert!(engine.skip_break(&clock).is_err(), "a focus is not a break");
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    wait(&clock, mins(15));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(5), false);
}

#[test]
fn skipping_when_idle_is_rejected() {
    let clock = fake();
    let mut engine = Engine::new();
    assert!(engine.skip_break(&clock).is_err());
    assert_idle(&engine.status(&clock));
}

// ---------------------------------------------------------------------------
// AC4 — long-break boundary
// ---------------------------------------------------------------------------

#[test]
fn only_the_last_focus_of_a_session_is_followed_by_the_long_break() {
    for cycles in 1..=MAX_CYCLES {
        let clock = fake();
        let engine = started(preset(25, 5, 15, cycles), LockKind::None, &clock);
        for cycle in 1..=cycles {
            assert_running(&engine.status(&clock), Phase::Focus, cycle, mins(25), false);
            wait(&clock, mins(25));
            if cycle < cycles {
                assert_running(
                    &engine.status(&clock),
                    Phase::ShortBreak,
                    cycle,
                    mins(5),
                    false,
                );
                wait(&clock, mins(5));
            } else {
                assert_running(
                    &engine.status(&clock),
                    Phase::LongBreak,
                    cycle,
                    mins(15),
                    false,
                );
                wait(&clock, mins(15));
            }
        }
        assert_idle(&engine.status(&clock));
    }
}

#[test]
fn one_cycle_session_goes_from_focus_straight_to_the_long_break() {
    let clock = fake();
    let engine = started(preset(25, 5, 15, 1), LockKind::None, &clock);
    wait(&clock, mins(25));
    assert_running(&engine.status(&clock), Phase::LongBreak, 1, mins(15), false);
    wait(&clock, mins(15));
    assert_idle(&engine.status(&clock));
}

// ---------------------------------------------------------------------------
// AC5 — bounds (#15)
// ---------------------------------------------------------------------------

#[test]
fn default_preset_is_25_5_15_minutes_and_4_cycles() {
    let preset = PomodoroPreset::default();
    assert_eq!(preset.focus(), mins(25));
    assert_eq!(preset.short_break(), mins(5));
    assert_eq!(preset.long_break(), mins(15));
    assert_eq!(preset.cycles(), 4);
    assert_eq!(PomodoroPreset::new(25, 5, 15, 4), Ok(preset));
}

#[test]
fn limits_are_pinned() {
    assert_eq!(MIN_DURATION_MIN, 1);
    assert_eq!(MAX_FOCUS_MIN, 120);
    assert_eq!(MAX_SHORT_BREAK_MIN, 30);
    assert_eq!(MAX_LONG_BREAK_MIN, 60);
    assert_eq!(MIN_CYCLES, 1);
    assert_eq!(MAX_CYCLES, 8);
}

#[test]
fn preset_accepts_the_minimum_and_maximum_of_every_value() {
    let smallest = preset(1, 1, 1, 1);
    assert_eq!(
        (
            smallest.focus(),
            smallest.short_break(),
            smallest.long_break(),
            smallest.cycles()
        ),
        (mins(1), mins(1), mins(1), 1)
    );
    let largest = preset(120, 30, 60, 8);
    assert_eq!(
        (
            largest.focus(),
            largest.short_break(),
            largest.long_break(),
            largest.cycles()
        ),
        (mins(120), mins(30), mins(60), 8)
    );
}

#[test]
fn preset_rejects_zero_for_every_value() {
    let cases = [
        (PomodoroPreset::new(0, 5, 15, 4), PresetField::Focus),
        (PomodoroPreset::new(25, 0, 15, 4), PresetField::ShortBreak),
        (PomodoroPreset::new(25, 5, 0, 4), PresetField::LongBreak),
        (PomodoroPreset::new(25, 5, 15, 0), PresetField::Cycles),
    ];
    for (result, field) in cases {
        let error = result.expect_err("zero must be rejected");
        assert_eq!(error.field(), field);
    }
}

#[test]
fn preset_rejects_one_above_the_maximum_for_every_value() {
    let cases = [
        (PomodoroPreset::new(121, 5, 15, 4), PresetField::Focus),
        (PomodoroPreset::new(25, 31, 15, 4), PresetField::ShortBreak),
        (PomodoroPreset::new(25, 5, 61, 4), PresetField::LongBreak),
        (PomodoroPreset::new(25, 5, 15, 9), PresetField::Cycles),
    ];
    for (result, field) in cases {
        let error = result.expect_err("a value above its maximum must be rejected");
        assert_eq!(error.field(), field);
    }
}

#[test]
fn preset_rejects_the_largest_u32_for_every_value_without_panicking() {
    let cases = [
        (PomodoroPreset::new(u32::MAX, 5, 15, 4), PresetField::Focus),
        (
            PomodoroPreset::new(25, u32::MAX, 15, 4),
            PresetField::ShortBreak,
        ),
        (
            PomodoroPreset::new(25, 5, u32::MAX, 4),
            PresetField::LongBreak,
        ),
        (
            PomodoroPreset::new(25, 5, 15, u32::MAX),
            PresetField::Cycles,
        ),
    ];
    for (result, field) in cases {
        let error = result.expect_err("u32::MAX must be rejected");
        assert_eq!(error.field(), field);
    }
}

#[test]
fn preset_error_is_a_standard_error_with_a_message() {
    let error = PomodoroPreset::new(121, 5, 15, 4).expect_err("out of range");
    let as_std: &dyn std::error::Error = &error;
    assert!(!as_std.to_string().is_empty());
}

#[test]
fn longest_possible_session_runs_to_the_end_without_panicking() {
    let clock = fake();
    let engine = started(
        preset(
            MAX_FOCUS_MIN,
            MAX_SHORT_BREAK_MIN,
            MAX_LONG_BREAK_MIN,
            MAX_CYCLES,
        ),
        LockKind::UntilTimerEnds,
        &clock,
    );
    // 8 × 120 + 7 × 30 + 60 minutes = 20 h 30 min.
    wait(&clock, mins(8 * 120 + 7 * 30 + 60) - Duration::from_secs(1));
    assert_running(
        &engine.status(&clock),
        Phase::LongBreak,
        8,
        Duration::from_secs(1),
        false,
    );
    wait(&clock, Duration::from_secs(1));
    assert_idle(&engine.status(&clock));
}

// ---------------------------------------------------------------------------
// AC6 — time comes from the monotonic reading only
// ---------------------------------------------------------------------------

#[test]
fn wall_clock_jumping_a_day_forward_neither_ends_nor_shortens_a_phase() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(10));

    clock.set_wall(wall_plus(&clock, DAY));
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    wait(&clock, mins(15) - Duration::from_secs(1));
    assert_running(
        &engine.status(&clock),
        Phase::Focus,
        1,
        Duration::from_secs(1),
        false,
    );
}

#[test]
fn wall_clock_jumping_backwards_neither_extends_nor_resets_a_phase() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(10));

    clock.set_wall(wall_minus(&clock, DAY));
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    wait(&clock, mins(15));
    assert_running(&engine.status(&clock), Phase::ShortBreak, 1, mins(5), false);
}

#[test]
fn wall_clock_changes_while_paused_change_nothing() {
    let clock = fake();
    let mut engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(10));
    engine.pause(&clock).expect("pause");

    clock.set_wall(wall_plus(&clock, DAY));
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), true);
    clock.set_wall(wall_minus(&clock, DAY + DAY));
    engine.resume(&clock).expect("resume");
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(15), false);

    wait(&clock, mins(5));
    assert_running(&engine.status(&clock), Phase::Focus, 1, mins(10), false);
}

// ---------------------------------------------------------------------------
// AC7 — late observation
// ---------------------------------------------------------------------------

#[test]
fn late_observation_advances_through_every_elapsed_phase() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);

    // Unobserved: focus 1, short break 1, then 10 minutes of focus 2.
    wait(&clock, mins(25 + 5 + 10));
    assert_running(&engine.status(&clock), Phase::Focus, 2, mins(15), false);

    // Unobserved: rest of focus 2, break 2, focus 3, break 3, 1 min of focus 4.
    wait(&clock, mins(15 + 5 + 25 + 5 + 1));
    assert_running(&engine.status(&clock), Phase::Focus, 4, mins(24), false);
}

#[test]
fn late_observation_after_the_session_ended_is_idle() {
    let clock = fake();
    let engine = started(PomodoroPreset::default(), LockKind::None, &clock);
    wait(&clock, mins(4 * 25 + 3 * 5 + 15) + mins(1));
    assert_idle(&engine.status(&clock));
}
