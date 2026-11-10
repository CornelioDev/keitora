// SPDX-License-Identifier: MIT OR Apache-2.0

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

/// A clock whose monotonic reading is set by the test, to any instant,
/// earlier ones included.
struct ManualClock(Cell<MonotonicInstant>);

impl ManualClock {
    fn at(since_origin: Duration) -> Self {
        Self(Cell::new(MonotonicInstant::from_duration_since_origin(
            since_origin,
        )))
    }

    fn set(&self, since_origin: Duration) {
        self.0
            .set(MonotonicInstant::from_duration_since_origin(since_origin));
    }
}

impl Clock for ManualClock {
    fn monotonic(&self) -> MonotonicInstant {
        self.0.get()
    }

    fn wall(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

fn mins(n: u64) -> Duration {
    Duration::from_secs(n.checked_mul(60).expect("minutes fit in a Duration"))
}

fn position(phase: Phase, cycle: u32, remaining: Duration) -> Option<Position> {
    Some(Position {
        phase,
        cycle,
        remaining,
    })
}

fn running(preset: PomodoroPreset, lock: LockKind, clock: &ManualClock) -> Engine {
    let mut engine = Engine::new();
    engine.start(preset, lock, clock).expect("start");
    engine
}

// --- PomodoroPreset and PresetError -----------------------------------------

#[test]
fn the_first_value_out_of_range_is_the_one_reported() {
    let error = PomodoroPreset::new(0, 0, 0, 0).expect_err("all out of range");
    assert_eq!(error.field(), PresetField::Focus);
    let error = PomodoroPreset::new(25, 31, 0, 0).expect_err("three out of range");
    assert_eq!(error.field(), PresetField::ShortBreak);
    assert_eq!(error.value(), 31);
}

#[test]
fn preset_error_messages_name_the_field_its_range_and_the_value() {
    let message =
        |result: Result<PomodoroPreset, PresetError>| result.expect_err("out of range").to_string();
    assert_eq!(
        message(PomodoroPreset::new(121, 5, 15, 4)),
        "the focus duration must be 1 to 120 minutes, not 121"
    );
    assert_eq!(
        message(PomodoroPreset::new(25, 0, 15, 4)),
        "the short break duration must be 1 to 30 minutes, not 0"
    );
    assert_eq!(
        message(PomodoroPreset::new(25, 5, 61, 4)),
        "the long break duration must be 1 to 60 minutes, not 61"
    );
    assert_eq!(
        message(PomodoroPreset::new(25, 5, 15, 9)),
        "the number of cycles must be 1 to 8, not 9"
    );
}

#[test]
fn durations_of_the_largest_minutes_do_not_overflow() {
    assert_eq!(
        minutes(u32::MAX),
        Duration::from_secs(u64::from(u32::MAX) * 60)
    );
}

// --- locate -----------------------------------------------------------------

#[test]
fn locate_puts_each_boundary_in_the_phase_that_starts_there() {
    let preset = PomodoroPreset::default();
    let at = |active| locate(&preset, 1, active);

    assert_eq!(at(Duration::ZERO), position(Phase::Focus, 1, mins(25)));
    assert_eq!(at(mins(25)), position(Phase::ShortBreak, 1, mins(5)));
    assert_eq!(at(mins(30)), position(Phase::Focus, 2, mins(25)));
    assert_eq!(at(mins(90)), position(Phase::Focus, 4, mins(25)));
    assert_eq!(at(mins(115)), position(Phase::LongBreak, 4, mins(15)));
    assert_eq!(
        at(mins(130) - Duration::from_nanos(1)),
        position(Phase::LongBreak, 4, Duration::from_nanos(1))
    );
    assert_eq!(at(mins(130)), None);
}

#[test]
fn locate_counts_from_the_cycle_the_schedule_restarted_at() {
    let preset = PomodoroPreset::default();
    assert_eq!(
        locate(&preset, 3, Duration::ZERO),
        position(Phase::Focus, 3, mins(25))
    );
    assert_eq!(
        locate(&preset, 3, mins(30)),
        position(Phase::Focus, 4, mins(25))
    );
    assert_eq!(
        locate(&preset, 4, mins(25)),
        position(Phase::LongBreak, 4, mins(15))
    );
    assert_eq!(locate(&preset, 4, mins(40)), None);
}

#[test]
fn locate_handles_the_longest_active_time_without_panicking() {
    let longest = PomodoroPreset::new(
        MAX_FOCUS_MIN,
        MAX_SHORT_BREAK_MIN,
        MAX_LONG_BREAK_MIN,
        MAX_CYCLES,
    )
    .expect("in range");
    assert_eq!(locate(&longest, 1, Duration::MAX), None);
    assert_eq!(locate(&PomodoroPreset::default(), 1, Duration::MAX), None);
}

// --- Engine -----------------------------------------------------------------

#[test]
fn a_default_engine_is_idle_with_cycle_zero_and_no_time_left() {
    let clock = ManualClock::at(Duration::ZERO);
    let status = Engine::default().status(&clock);
    assert_eq!(status, Status::IDLE);
    assert_eq!(
        (status.cycle(), status.remaining(), status.is_paused()),
        (0, Duration::ZERO, false)
    );
}

#[test]
fn each_misuse_returns_its_own_error() {
    let clock = ManualClock::at(mins(1));
    let mut idle = Engine::new();
    assert_eq!(idle.pause(&clock), Err(EngineError::NoSession));
    assert_eq!(idle.resume(&clock), Err(EngineError::NoSession));
    assert_eq!(idle.skip_break(&clock), Err(EngineError::NoSession));
    assert_eq!(idle.stop(&clock), Err(EngineError::NoSession));

    let mut engine = running(PomodoroPreset::default(), LockKind::None, &clock);
    assert_eq!(
        engine.start(PomodoroPreset::default(), LockKind::None, &clock),
        Err(EngineError::AlreadyRunning)
    );
    assert_eq!(engine.resume(&clock), Err(EngineError::NotPaused));
    assert_eq!(engine.skip_break(&clock), Err(EngineError::NotABreak));
    engine.pause(&clock).expect("pause");
    assert_eq!(engine.pause(&clock), Err(EngineError::AlreadyPaused));
    assert_eq!(
        engine.start(PomodoroPreset::default(), LockKind::None, &clock),
        Err(EngineError::AlreadyRunning)
    );

    let mut strict = running(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock);
    assert_eq!(strict.pause(&clock), Err(EngineError::StrictSession));
    assert_eq!(strict.resume(&clock), Err(EngineError::NotPaused));
}

#[test]
fn error_messages_are_not_empty() {
    let errors = [
        EngineError::AlreadyRunning,
        EngineError::NoSession,
        EngineError::AlreadyPaused,
        EngineError::NotPaused,
        EngineError::NotABreak,
        EngineError::StrictSession,
    ];
    for error in errors {
        assert!(!error.to_string().is_empty(), "{error:?}");
    }
}

#[test]
fn a_session_that_ended_unobserved_counts_as_no_session() {
    let clock = ManualClock::at(Duration::ZERO);
    let mut engine = running(
        PomodoroPreset::new(1, 1, 1, 1).expect("in range"),
        LockKind::None,
        &clock,
    );
    clock.set(mins(2));

    assert_eq!(engine.pause(&clock), Err(EngineError::NoSession));
    assert_eq!(engine.resume(&clock), Err(EngineError::NoSession));
    assert_eq!(engine.skip_break(&clock), Err(EngineError::NoSession));
    assert_eq!(engine.stop(&clock), Err(EngineError::NoSession));
    assert_eq!(engine.status(&clock), Status::IDLE);
    engine
        .start(PomodoroPreset::default(), LockKind::None, &clock)
        .expect("a session that is over is replaced");
    assert_eq!(engine.status(&clock).remaining(), mins(25));
}

#[test]
fn skipping_a_short_break_while_paused_stays_paused_at_the_next_focus() {
    let clock = ManualClock::at(Duration::ZERO);
    let mut engine = running(PomodoroPreset::default(), LockKind::None, &clock);
    clock.set(mins(26));
    engine.pause(&clock).expect("pause");
    engine.skip_break(&clock).expect("skip");

    clock.set(mins(60));
    let status = engine.status(&clock);
    assert_eq!(
        (
            status.phase(),
            status.cycle(),
            status.remaining(),
            status.is_paused()
        ),
        (Phase::Focus, 2, mins(25), true)
    );

    engine.resume(&clock).expect("resume");
    clock.set(mins(85));
    assert_eq!(engine.status(&clock).phase(), Phase::ShortBreak);
    assert_eq!(engine.status(&clock).cycle(), 2);
}

#[test]
fn skipping_the_long_break_while_paused_ends_the_session() {
    let clock = ManualClock::at(Duration::ZERO);
    let mut engine = running(
        PomodoroPreset::new(25, 5, 15, 1).expect("in range"),
        LockKind::None,
        &clock,
    );
    clock.set(mins(30));
    engine.pause(&clock).expect("pause");
    engine.skip_break(&clock).expect("skip");
    assert_eq!(engine.status(&clock), Status::IDLE);
}

#[test]
fn a_clock_reading_earlier_than_the_start_counts_as_no_time_passing() {
    let clock = ManualClock::at(mins(100));
    let mut engine = running(PomodoroPreset::default(), LockKind::None, &clock);

    clock.set(mins(40));
    assert_eq!(engine.status(&clock).remaining(), mins(25));
    engine.pause(&clock).expect("pause");
    assert_eq!(engine.status(&clock).remaining(), mins(25));
    engine.resume(&clock).expect("resume");

    clock.set(mins(50));
    assert_eq!(engine.status(&clock).remaining(), mins(15));
}

#[test]
fn the_farthest_clock_reading_ends_the_session_without_panicking() {
    let clock = ManualClock::at(Duration::ZERO);
    let mut engine = running(PomodoroPreset::default(), LockKind::None, &clock);
    clock.set(mins(10));
    engine.pause(&clock).expect("pause");
    engine.resume(&clock).expect("resume");

    clock.set(Duration::MAX);
    assert_eq!(engine.status(&clock), Status::IDLE);
}

#[test]
fn lock_validation_bounds_the_random_text_length() {
    let bad = LockError { len: 0 };
    assert_eq!(LockKind::RandomText { len: 0 }.validate(), Err(bad));
    assert_eq!(bad.value(), 0);
    assert!(bad.to_string().contains('0'));
    assert_eq!(
        LockKind::RandomText {
            len: MAX_RANDOM_TEXT_LEN + 1
        }
        .validate(),
        Err(LockError {
            len: MAX_RANDOM_TEXT_LEN + 1
        })
    );
    assert_eq!(LockKind::RandomText { len: 8 }.validate(), Ok(()));
}

#[test]
fn start_reports_an_invalid_lock_before_anything_else() {
    let clock = ManualClock::at(Duration::ZERO);
    let mut engine = running(PomodoroPreset::default(), LockKind::None, &clock);
    assert_eq!(
        engine.start(
            PomodoroPreset::default(),
            LockKind::RandomText { len: 0 },
            &clock
        ),
        Err(EngineError::InvalidLock(LockError { len: 0 }))
    );
}

#[test]
fn start_over_a_strict_session_is_a_strict_refusal() {
    let clock = ManualClock::at(Duration::ZERO);
    let mut engine = running(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock);
    assert_eq!(
        engine.start(PomodoroPreset::default(), LockKind::None, &clock),
        Err(EngineError::StrictSession)
    );
}

#[test]
fn strict_refusals_leave_a_paused_free_session_alone_and_messages_exist() {
    let clock = ManualClock::at(Duration::ZERO);
    let mut engine = running(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock);
    assert_eq!(engine.stop(&clock), Err(EngineError::StrictSession));
    assert_eq!(engine.skip_break(&clock), Err(EngineError::StrictSession));
    assert_eq!(engine.can_stop(&clock), Err(LockReason::UntilTimerEnds));
    assert!(!LockReason::UntilTimerEnds.to_string().is_empty());
    assert!(
        !EngineError::InvalidLock(LockError { len: 0 })
            .to_string()
            .is_empty()
    );
}

// --- Serde of PomodoroPreset and LockKind -----------------------------------

#[test]
fn a_preset_reads_back_what_it_writes() {
    let preset = PomodoroPreset::new(50, 10, 30, 3).expect("in range");
    let json = serde_json::to_value(&preset).expect("serializes");
    assert_eq!(
        json,
        serde_json::json!({
            "focus_min": 50,
            "short_break_min": 10,
            "long_break_min": 30,
            "cycles": 3,
        })
    );
    let back: PomodoroPreset = serde_json::from_value(json).expect("deserializes");
    assert_eq!(back, preset);
}

#[test]
fn a_preset_out_of_range_is_rejected_with_the_message_of_new() {
    let json = r#"{ "focus_min": 25, "short_break_min": 31, "long_break_min": 15, "cycles": 4 }"#;
    let error = serde_json::from_str::<PomodoroPreset>(json).expect_err("out of range");
    let expected = PomodoroPreset::new(25, 31, 15, 4).expect_err("out of range");
    assert!(error.to_string().contains(&expected.to_string()), "{error}");
}

#[test]
fn every_lock_kind_reads_back_what_it_writes() {
    for lock in [
        LockKind::None,
        LockKind::UntilTimerEnds,
        LockKind::RandomText { len: 8 },
        // Out of range: deserializing doesn't check it, `validate` does.
        LockKind::RandomText { len: 0 },
    ] {
        let text = serde_json::to_string(&lock).expect("serializes");
        let back: LockKind = serde_json::from_str(&text).expect("deserializes");
        assert_eq!(back, lock, "{text}");
    }
}

#[test]
fn only_a_random_text_lock_has_a_length() {
    for json in [
        r#"{ "kind": "none", "len": 8 }"#,
        r#"{ "kind": "until_timer_ends", "len": 8 }"#,
        r#"{ "kind": "random_text" }"#,
        r#"{ "kind": "random_text", "len": null }"#,
    ] {
        assert!(
            serde_json::from_str::<LockKind>(json).is_err(),
            "{json} must be rejected"
        );
    }
    let lock: LockKind =
        serde_json::from_str(r#"{ "kind": "until_timer_ends", "len": null, "other": [1, 2] }"#)
            .expect("a null length and unknown fields are skipped");
    assert_eq!(lock, LockKind::UntilTimerEnds);
}

// --- Snapshots ------------------------------------------------------------------

/// A snapshot with a session, built directly.
fn state(lock: LockKind, first_cycle: u32, active: Duration, paused: bool) -> EngineState {
    EngineState {
        session: Some(SessionState {
            preset: PomodoroPreset::default(),
            lock,
            first_cycle,
            active,
            paused,
        }),
    }
}

#[test]
fn snapshot_of_an_idle_or_ended_engine_has_no_session() {
    let clock = ManualClock::at(mins(3));
    assert_eq!(
        Engine::new().snapshot(&clock),
        EngineState { session: None }
    );
    let engine = running(PomodoroPreset::default(), LockKind::None, &clock);
    clock.set(mins(3 + 130));
    assert_eq!(engine.snapshot(&clock), EngineState { session: None });
}

#[test]
fn snapshot_holds_the_active_time_and_not_the_clock_reading() {
    let clock = ManualClock::at(mins(1000));
    let mut engine = running(PomodoroPreset::default(), LockKind::None, &clock);
    clock.set(mins(1010));
    engine.pause(&clock).expect("pause");
    clock.set(mins(2000));
    assert_eq!(
        engine.snapshot(&clock),
        state(LockKind::None, 1, mins(10), true)
    );
}

#[test]
fn restore_continues_from_the_stored_active_time_on_any_clock() {
    let clock = ManualClock::at(Duration::MAX);
    let engine =
        Engine::restore(state(LockKind::None, 2, mins(26), false), &clock).expect("restores");
    assert_eq!(
        engine.status(&clock),
        Status {
            phase: Phase::ShortBreak,
            cycle: 2,
            remaining: mins(4),
            paused: false,
        }
    );

    let clock = ManualClock::at(mins(7));
    let engine = Engine::restore(EngineState { session: None }, &clock).expect("restores");
    assert_eq!(engine.status(&clock), Status::IDLE);
}

#[test]
fn restore_reports_each_problem_with_its_own_error() {
    let clock = ManualClock::at(Duration::ZERO);
    let cases = [
        (
            state(LockKind::RandomText { len: 0 }, 9, mins(500), true),
            RestoreError::InvalidLock(LockError { len: 0 }),
        ),
        (
            state(LockKind::UntilTimerEnds, 0, Duration::ZERO, true),
            RestoreError::FirstCycleOutOfRange {
                first_cycle: 0,
                cycles: 4,
            },
        ),
        (
            state(LockKind::None, 5, Duration::ZERO, false),
            RestoreError::FirstCycleOutOfRange {
                first_cycle: 5,
                cycles: 4,
            },
        ),
        (
            state(LockKind::UntilTimerEnds, 2, mins(500), true),
            RestoreError::PausedStrictSession,
        ),
        (
            state(LockKind::UntilTimerEnds, 2, mins(500), false),
            RestoreError::StrictSessionSkippedCycles { first_cycle: 2 },
        ),
        (
            state(LockKind::UntilTimerEnds, 1, mins(130), false),
            RestoreError::SessionEnded,
        ),
        (
            state(LockKind::None, 4, mins(40), true),
            RestoreError::SessionEnded,
        ),
    ];
    for (state, expected) in cases {
        let error = Engine::restore(state.clone(), &clock).expect_err("rejected");
        assert_eq!(error, expected, "{state:?}");
        assert!(!error.to_string().is_empty(), "{error:?}");
    }
}
