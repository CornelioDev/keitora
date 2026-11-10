// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-1.6: the strict-mode policy.
//!
//! # Expected public API
//!
//! Everything lives in `keitora_core::engine` (T-1.6 may move the types and
//! re-export them there):
//!
//! - `LockKind` (`#[non_exhaustive]`): `None`, `UntilTimerEnds` and the new
//!   `RandomText { len: u32 }`.
//!   - `LockKind::validate(&self) -> Result<(), LockError>`: `RandomText` with
//!     `len` outside `MIN_RANDOM_TEXT_LEN..=MAX_RANDOM_TEXT_LEN` is rejected,
//!     never clamped; `None` and `UntilTimerEnds` are always valid.
//! - `MIN_RANDOM_TEXT_LEN` and `MAX_RANDOM_TEXT_LEN`: `pub const u32`. The
//!   minimum is at least 1; the maximum is a defined, modest number (the tests
//!   only require it to be at most 1024).
//! - `LockError`: `Debug`, `Clone`, `PartialEq`, `std::error::Error` with a
//!   non-empty message.
//! - `Engine::start` validates the lock: an invalid one is an `Err` and the
//!   engine stays idle (the service can't forget to call `validate`).
//! - `LockReason`: `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`,
//!   `std::error::Error` with a non-empty message. `LockReason::UntilTimerEnds`
//!   is the reason a strict session gives.
//! - `Engine::can_stop(&self, &clock) -> Result<(), LockReason>`: the
//!   `can_stop(&session, now)` of the roadmap; the engine owns the session and
//!   reads `now` from the clock. `Ok` when stopping would be allowed (no
//!   session, session over, or a session that isn't strict).
//! - `Engine::can_edit_blocklist(&self, &clock) -> Result<(), LockReason>`:
//!   the blocklist-edit rule (#18). `Err(LockReason::UntilTimerEnds)` while a
//!   strict session is active (every phase of it, focus and breaks); `Ok`
//!   otherwise, and again the instant the session's timer ends.
//! - `Engine::stop`, `Engine::skip_break` and `Engine::pause` refuse a strict
//!   session with `EngineError::StrictSession` and leave the engine unchanged.
//!   Any skip is refused in a strict session, short or long (maintainer
//!   decision, 2026-11-10).
//!
//! # Behavior pinned (maintainer decisions)
//!
//! - Only `UntilTimerEnds` is strict. `RandomText` is friction, not a lock:
//!   the rules for strict sessions don't apply to it. Core has no random
//!   source, so generating and checking the typed text belongs to the layer
//!   above; to the engine a `RandomText` session is stoppable, pausable and
//!   editable like a `None` one.
//! - A strict session is active until its timer ends (all phases, including
//!   the long break). From that instant stop and edits are allowed again.
//!
//! # Criteria covered
//!
//! - **AC1** — an active strict session rejects stop:
//!   `strict_session_rejects_can_stop_in_every_phase`,
//!   `strict_session_rejects_stop_in_every_phase_and_keeps_running`.
//! - **AC2** — rejects pause (T-1.3 behavior kept):
//!   `strict_session_still_rejects_pause_and_can_stop_says_why`.
//! - **AC3** — rejects shortening (skip, restart):
//!   `strict_session_rejects_skipping_the_short_break`,
//!   `strict_session_rejects_skipping_the_long_break_and_does_not_end_early`,
//!   `strict_session_rejects_skipping_during_focus`,
//!   `strict_session_rejects_starting_a_replacement_session`,
//!   `failed_attempts_on_a_strict_session_do_not_move_its_schedule`.
//! - **AC4** (#18) — rejects blocklist edits, and the end unlocks them:
//!   `strict_session_rejects_blocklist_edits_in_every_phase`,
//!   `strict_session_unlocks_stop_and_edits_exactly_when_the_timer_ends`,
//!   `stop_and_edits_are_allowed_when_idle_or_never_started`.
//! - **AC5** — only `UntilTimerEnds` is strict:
//!   `non_strict_sessions_can_stop_pause_skip_and_edit`,
//!   `random_text_session_is_not_strict`.
//! - **AC6** (#15) — `RandomText { len }` has a maximum:
//!   `random_text_length_limits_are_pinned`,
//!   `random_text_accepts_the_minimum_and_maximum_length`,
//!   `random_text_rejects_zero_one_above_maximum_and_u32_max`,
//!   `other_locks_are_always_valid`,
//!   `lock_error_is_a_standard_error_with_a_message`,
//!   `start_rejects_an_invalid_lock_and_stays_idle`,
//!   `start_accepts_random_text_at_the_limits`.
//! - **AC7** — the reasons are usable errors:
//!   `lock_reason_is_a_standard_error_with_a_message`.
//!
//! Not covered here: checking the typed text of `RandomText` (belongs to the
//! service/UI layer, which owns the random source); persistence (T-1.7);
//! service-side message validation (F2).

#![cfg(test)]

use std::time::{Duration, UNIX_EPOCH};

use keitora_core::clock::FakeClock;
use keitora_core::engine::{
    Engine, EngineError, LockError, LockKind, LockReason, MAX_RANDOM_TEXT_LEN, MIN_RANDOM_TEXT_LEN,
    Phase, PomodoroPreset,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fake() -> FakeClock {
    FakeClock::new(UNIX_EPOCH)
}

fn mins(n: u64) -> Duration {
    Duration::from_secs(n.checked_mul(60).expect("minutes fit in a Duration"))
}

fn wait(clock: &FakeClock, by: Duration) {
    clock.advance(by).expect("the fake clock advances");
}

/// A running session with the default preset (25 / 5 / 15, 4 cycles: 130
/// minutes in all).
fn started(lock: LockKind, clock: &FakeClock) -> Engine {
    let mut engine = Engine::new();
    engine
        .start(PomodoroPreset::default(), lock, clock)
        .expect("starting from idle succeeds");
    engine
}

/// The whole default session, in minutes.
const SESSION_MIN: u64 = 130;

/// Minutes into the default session at which each phase is current, with the
/// phase expected there.
const PHASE_POINTS: [(u64, Phase); 3] = [
    (10, Phase::Focus),
    (27, Phase::ShortBreak),
    (SESSION_MIN - 5, Phase::LongBreak),
];

/// Walks the clock to `minute` of the session and checks the phase there.
fn go_to(engine: &Engine, clock: &FakeClock, from_min: u64, minute: u64, phase: Phase) {
    let delta = minute.checked_sub(from_min).expect("minutes go forward");
    wait(clock, mins(delta));
    assert_eq!(
        engine.status(clock).phase(),
        phase,
        "phase at minute {minute}"
    );
}

fn assert_strict_refusal<T: std::fmt::Debug>(result: Result<T, EngineError>, what: &str) {
    assert!(
        matches!(result, Err(EngineError::StrictSession)),
        "{what} on a strict session must be Err(StrictSession), got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// AC1 — stop
// ---------------------------------------------------------------------------

#[test]
fn strict_session_rejects_can_stop_in_every_phase() {
    let clock = fake();
    let engine = started(LockKind::UntilTimerEnds, &clock);
    assert_eq!(engine.can_stop(&clock), Err(LockReason::UntilTimerEnds));

    let mut at = 0;
    for (minute, phase) in PHASE_POINTS {
        go_to(&engine, &clock, at, minute, phase);
        at = minute;
        assert_eq!(
            engine.can_stop(&clock),
            Err(LockReason::UntilTimerEnds),
            "can_stop during {phase:?}"
        );
    }
}

#[test]
fn strict_session_rejects_stop_in_every_phase_and_keeps_running() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);

    let mut at = 0;
    for (minute, phase) in PHASE_POINTS {
        go_to(&engine, &clock, at, minute, phase);
        at = minute;
        let before = engine.status(&clock);
        assert_strict_refusal(engine.stop(&clock), "stop");
        assert_eq!(
            engine.status(&clock),
            before,
            "stop changed the engine in {phase:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC2 — pause
// ---------------------------------------------------------------------------

#[test]
fn strict_session_still_rejects_pause_and_can_stop_says_why() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(3));
    assert_strict_refusal(engine.pause(&clock), "pause");
    assert!(!engine.status(&clock).is_paused());
    assert_eq!(engine.can_stop(&clock), Err(LockReason::UntilTimerEnds));
}

// ---------------------------------------------------------------------------
// AC3 — shortening
// ---------------------------------------------------------------------------

#[test]
fn strict_session_rejects_skipping_the_short_break() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(27));
    let before = engine.status(&clock);
    assert_eq!(before.phase(), Phase::ShortBreak);

    assert_strict_refusal(engine.skip_break(&clock), "skip_break");
    assert_eq!(engine.status(&clock), before);
}

#[test]
fn strict_session_rejects_skipping_the_long_break_and_does_not_end_early() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(SESSION_MIN - 10));
    let before = engine.status(&clock);
    assert_eq!(before.phase(), Phase::LongBreak);
    assert_eq!(before.remaining(), mins(10));

    assert_strict_refusal(engine.skip_break(&clock), "skip_break");
    assert_eq!(engine.status(&clock), before);
    // The session still ends only when its timer does.
    wait(&clock, mins(9));
    assert_eq!(engine.status(&clock).phase(), Phase::LongBreak);
    wait(&clock, mins(1));
    assert_eq!(engine.status(&clock).phase(), Phase::Idle);
}

#[test]
fn strict_session_rejects_skipping_during_focus() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(1));
    let before = engine.status(&clock);
    assert!(engine.skip_break(&clock).is_err());
    assert_eq!(engine.status(&clock), before);
}

#[test]
fn strict_session_rejects_starting_a_replacement_session() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(10));
    let before = engine.status(&clock);

    let shorter = PomodoroPreset::new(1, 1, 1, 1).expect("in range");
    assert!(engine.start(shorter, LockKind::None, &clock).is_err());
    assert_eq!(engine.status(&clock), before);
    assert_eq!(engine.can_stop(&clock), Err(LockReason::UntilTimerEnds));
}

#[test]
fn failed_attempts_on_a_strict_session_do_not_move_its_schedule() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);
    wait(&clock, mins(26));
    // Rejected calls in the middle must not restart, pause or shift anything.
    assert!(engine.stop(&clock).is_err());
    assert!(engine.pause(&clock).is_err());
    assert!(engine.skip_break(&clock).is_err());
    wait(&clock, mins(104));
    assert_eq!(engine.status(&clock).phase(), Phase::Idle, "ended on time");
}

// ---------------------------------------------------------------------------
// AC4 — blocklist edits (#18) and the end of the session
// ---------------------------------------------------------------------------

#[test]
fn strict_session_rejects_blocklist_edits_in_every_phase() {
    let clock = fake();
    let engine = started(LockKind::UntilTimerEnds, &clock);
    assert_eq!(
        engine.can_edit_blocklist(&clock),
        Err(LockReason::UntilTimerEnds)
    );

    let mut at = 0;
    for (minute, phase) in PHASE_POINTS {
        go_to(&engine, &clock, at, minute, phase);
        at = minute;
        assert_eq!(
            engine.can_edit_blocklist(&clock),
            Err(LockReason::UntilTimerEnds),
            "edit during {phase:?}"
        );
    }
}

#[test]
fn strict_session_unlocks_stop_and_edits_exactly_when_the_timer_ends() {
    let clock = fake();
    let mut engine = started(LockKind::UntilTimerEnds, &clock);

    wait(&clock, mins(SESSION_MIN) - Duration::from_nanos(1));
    assert_eq!(engine.can_stop(&clock), Err(LockReason::UntilTimerEnds));
    assert_eq!(
        engine.can_edit_blocklist(&clock),
        Err(LockReason::UntilTimerEnds)
    );

    wait(&clock, Duration::from_nanos(1));
    assert_eq!(engine.status(&clock).phase(), Phase::Idle);
    assert_eq!(engine.can_stop(&clock), Ok(()));
    assert_eq!(engine.can_edit_blocklist(&clock), Ok(()));

    // A new session can start, and it locks again.
    engine
        .start(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock)
        .expect("a finished session doesn't block a new one");
    assert_eq!(engine.can_stop(&clock), Err(LockReason::UntilTimerEnds));
    assert_eq!(
        engine.can_edit_blocklist(&clock),
        Err(LockReason::UntilTimerEnds)
    );
}

#[test]
fn stop_and_edits_are_allowed_when_idle_or_never_started() {
    let clock = fake();
    let engine = Engine::new();
    assert_eq!(engine.can_stop(&clock), Ok(()));
    assert_eq!(engine.can_edit_blocklist(&clock), Ok(()));
}

// ---------------------------------------------------------------------------
// AC5 — only UntilTimerEnds is strict
// ---------------------------------------------------------------------------

#[test]
fn non_strict_sessions_can_stop_pause_skip_and_edit() {
    let clock = fake();
    let mut engine = started(LockKind::None, &clock);
    assert_eq!(engine.can_stop(&clock), Ok(()));
    assert_eq!(engine.can_edit_blocklist(&clock), Ok(()));

    engine.pause(&clock).expect("pause works");
    engine.resume(&clock).expect("resume works");
    wait(&clock, mins(27));
    engine.skip_break(&clock).expect("skip works");
    engine.stop(&clock).expect("stop works");
    assert_eq!(engine.status(&clock).phase(), Phase::Idle);
}

#[test]
fn random_text_session_is_not_strict() {
    let clock = fake();
    let lock = LockKind::RandomText {
        len: MIN_RANDOM_TEXT_LEN,
    };
    let mut engine = started(lock, &clock);

    assert_eq!(engine.can_stop(&clock), Ok(()));
    assert_eq!(engine.can_edit_blocklist(&clock), Ok(()));
    engine.pause(&clock).expect("a friction lock can be paused");
    engine.resume(&clock).expect("and resumed");
    wait(&clock, mins(27));
    engine.skip_break(&clock).expect("and its break skipped");
    engine.stop(&clock).expect("and stopped");
    assert_eq!(engine.status(&clock).phase(), Phase::Idle);
}

// ---------------------------------------------------------------------------
// AC6 — RandomText limits (#15)
// ---------------------------------------------------------------------------

#[test]
fn random_text_length_limits_are_pinned() {
    const {
        assert!(
            MIN_RANDOM_TEXT_LEN >= 1,
            "a zero-length text is no challenge"
        );
        assert!(MIN_RANDOM_TEXT_LEN <= MAX_RANDOM_TEXT_LEN);
        assert!(MAX_RANDOM_TEXT_LEN <= 1024, "the maximum must be modest");
    }
}

#[test]
fn random_text_accepts_the_minimum_and_maximum_length() {
    for len in [MIN_RANDOM_TEXT_LEN, MAX_RANDOM_TEXT_LEN] {
        assert_eq!(LockKind::RandomText { len }.validate(), Ok(()), "len {len}");
    }
}

#[test]
fn random_text_rejects_zero_one_above_maximum_and_u32_max() {
    let above = MAX_RANDOM_TEXT_LEN.checked_add(1).expect("fits in u32");
    let mut lens = vec![above, u32::MAX];
    if let Some(below) = MIN_RANDOM_TEXT_LEN.checked_sub(1) {
        lens.push(below);
    }
    for len in lens {
        assert!(
            LockKind::RandomText { len }.validate().is_err(),
            "len {len} must be rejected, not clamped"
        );
    }
}

#[test]
fn other_locks_are_always_valid() {
    assert_eq!(LockKind::None.validate(), Ok(()));
    assert_eq!(LockKind::UntilTimerEnds.validate(), Ok(()));
}

#[test]
fn lock_error_is_a_standard_error_with_a_message() {
    let error: LockError = LockKind::RandomText { len: u32::MAX }
        .validate()
        .expect_err("out of range");
    let as_std: &dyn std::error::Error = &error;
    assert!(!as_std.to_string().is_empty());
    assert_eq!(error.clone(), error);
}

#[test]
fn start_rejects_an_invalid_lock_and_stays_idle() {
    let clock = fake();
    let mut engine = Engine::new();
    for len in [0, MAX_RANDOM_TEXT_LEN.saturating_add(1), u32::MAX] {
        if (MIN_RANDOM_TEXT_LEN..=MAX_RANDOM_TEXT_LEN).contains(&len) {
            continue;
        }
        let result = engine.start(
            PomodoroPreset::default(),
            LockKind::RandomText { len },
            &clock,
        );
        assert!(result.is_err(), "len {len} must not start a session");
        assert_eq!(engine.status(&clock).phase(), Phase::Idle);
    }
}

#[test]
fn start_accepts_random_text_at_the_limits() {
    let clock = fake();
    for len in [MIN_RANDOM_TEXT_LEN, MAX_RANDOM_TEXT_LEN] {
        let mut engine = Engine::new();
        engine
            .start(
                PomodoroPreset::default(),
                LockKind::RandomText { len },
                &clock,
            )
            .expect("a valid length starts");
        assert_eq!(engine.status(&clock).phase(), Phase::Focus);
    }
}

// ---------------------------------------------------------------------------
// AC7 — reasons
// ---------------------------------------------------------------------------

#[test]
fn lock_reason_is_a_standard_error_with_a_message() {
    let reason = LockReason::UntilTimerEnds;
    let as_std: &dyn std::error::Error = &reason;
    assert!(!as_std.to_string().is_empty());
    let copy = reason;
    assert_eq!(reason, copy);
}
