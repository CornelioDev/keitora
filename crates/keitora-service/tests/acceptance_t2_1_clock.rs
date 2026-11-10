// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.1, security note (from the T-1.3 review,
//! ADR-0010): the real clock the service gives the engine.
//!
//! The binary, its console mode and its logs are tested in
//! `acceptance_t2_1.rs`.
//!
//! # Expected public API
//!
//! `keitora-service` gains a library target (`src/lib.rs`, crate
//! `keitora_service`) next to its binary, and the binary uses it. The tests
//! need only:
//!
//! - `keitora_service::clock::SystemClock`, the service's implementation of
//!   `keitora_core::clock::Clock`:
//!   - `SystemClock::new() -> SystemClock`;
//!   - `monotonic()` reads a 64-bit monotonic source that never wraps or goes
//!     backwards (`std::time::Instant`, which is QPC on Windows, or
//!     `QueryUnbiasedInterruptTime` through `keitora-sys`; never
//!     `GetTickCount`). The origin is the developer's choice;
//!   - `wall()` is the system's calendar time;
//!   - `Send + Sync + Debug`, so one instance can be shared by the whole
//!     service.
//!
//! Where the clock comes from inside the service (one instance, created by
//! the service itself, never from a client, never `FakeClock`) is for review:
//! the tests can't see it. Nothing else about the library is fixed here.
//!
//! # Criteria covered
//!
//! - **CL1** — the monotonic reading never goes backwards, also when one
//!   instance is shared between threads:
//!   `monotonic_reading_never_goes_backwards_over_many_reads`,
//!   `monotonic_reading_never_goes_backwards_across_threads_sharing_one_clock`.
//! - **CL2** — the monotonic reading follows real time:
//!   `monotonic_reading_advances_with_real_time`.
//! - **CL3** — the wall reading is the system's calendar time:
//!   `wall_reading_follows_the_system_time`.
//! - **CL4** — the engine runs on it, as the service will run it:
//!   `engine_runs_a_session_on_the_system_clock`,
//!   `system_clock_can_be_shared_as_a_clock_trait_object`.
//!
//! # Verified by review, not here
//!
//! - The source is 64-bit and can't wrap (no `GetTickCount`), and the
//!   service builds exactly one instance and passes it to the engine; no IPC
//!   message, argument or setting can choose or move it.
//! - The documentation says whether time asleep or hibernated counts toward
//!   a session's active time with the chosen source (`Instant` keeps counting
//!   through sleep on Windows; `QueryUnbiasedInterruptTime` doesn't).
//! - The engine changes only through its methods, never by being replaced
//!   with `Engine::default()` or `Engine::new()` while a session runs.

#![cfg(test)]

use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use keitora_core::clock::{Clock, MonotonicInstant};
use keitora_core::engine::{Engine, LockKind, Phase, PomodoroPreset};
use keitora_service::clock::SystemClock;

/// How long the tests sleep to see real time pass.
const NAP: Duration = Duration::from_millis(200);

/// What a reading may lose to the source's resolution: Windows' interrupt
/// time moves in timer ticks of about 15.6 ms, and a measurement spans two
/// readings.
const RESOLUTION_SLACK: Duration = Duration::from_millis(50);

/// An upper bound no healthy test run reaches, even on a slow CI runner.
const FAR_TOO_LONG: Duration = Duration::from_secs(60);

/// How far the clock's wall reading may be from `SystemTime::now()`.
const WALL_TOLERANCE: Duration = Duration::from_secs(5);

/// Compiles only if `T` can be shared between threads.
fn assert_send_sync<T: Send + Sync>() {}

/// Compiles only if `T` implements `Debug`.
fn assert_debug<T: std::fmt::Debug>() {}

/// Takes `count` monotonic readings and fails the test if one is earlier than
/// the one before it. Returns the last reading.
fn read_many<C: Clock + ?Sized>(clock: &C, count: usize) -> MonotonicInstant {
    let mut previous = clock.monotonic();
    for _ in 0..count {
        let current = clock.monotonic();
        assert!(
            current >= previous,
            "the monotonic reading went backwards: {previous:?} then {current:?}"
        );
        previous = current;
    }
    previous
}

/// Distance between two wall times, whichever is later.
fn wall_distance(a: SystemTime, b: SystemTime) -> Duration {
    a.duration_since(b)
        .or_else(|_| b.duration_since(a))
        .expect("one of the two is the later")
}

// ---------------------------------------------------------------------------
// CL1 — never backwards
// ---------------------------------------------------------------------------

#[test]
fn monotonic_reading_never_goes_backwards_over_many_reads() {
    let clock = SystemClock::new();
    read_many(&clock, 200_000);
}

#[test]
fn monotonic_reading_never_goes_backwards_across_threads_sharing_one_clock() {
    assert_send_sync::<SystemClock>();
    assert_debug::<SystemClock>();
    let clock = Arc::new(SystemClock::new());
    let before = clock.monotonic();

    let workers: Vec<_> = (0..4)
        .map(|_| {
            let clock = Arc::clone(&clock);
            thread::spawn(move || {
                let first = clock.monotonic();
                let last = read_many(clock.as_ref(), 50_000);
                (first, last)
            })
        })
        .collect();
    let mut latest_in_threads = before;
    for worker in workers {
        let (first, last) = worker.join().expect("a reader thread failed");
        assert!(
            first >= before,
            "a thread read an instant earlier than one read before it started"
        );
        latest_in_threads = latest_in_threads.max(last);
    }

    let after = clock.monotonic();
    assert!(
        after >= latest_in_threads,
        "a reading after the threads ended is earlier than one they took"
    );
}

// ---------------------------------------------------------------------------
// CL2 — follows real time
// ---------------------------------------------------------------------------

#[test]
fn monotonic_reading_advances_with_real_time() {
    let clock = SystemClock::new();
    let start = clock.monotonic();
    thread::sleep(NAP);
    let elapsed = clock
        .monotonic()
        .checked_duration_since(start)
        .expect("the monotonic reading went backwards");

    assert!(
        elapsed + RESOLUTION_SLACK >= NAP,
        "slept {NAP:?} but the clock moved only {elapsed:?}"
    );
    assert!(
        elapsed < FAR_TOO_LONG,
        "slept {NAP:?} but the clock moved {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// CL3 — wall reading
// ---------------------------------------------------------------------------

#[test]
fn wall_reading_follows_the_system_time() {
    let clock = SystemClock::new();

    let distance = wall_distance(clock.wall(), SystemTime::now());
    assert!(
        distance <= WALL_TOLERANCE,
        "the wall reading is {distance:?} away from the system time"
    );

    thread::sleep(NAP);
    let distance = wall_distance(clock.wall(), SystemTime::now());
    assert!(
        distance <= WALL_TOLERANCE,
        "the wall reading drifted {distance:?} away from the system time"
    );
}

// ---------------------------------------------------------------------------
// CL4 — the engine runs on it
// ---------------------------------------------------------------------------

#[test]
fn engine_runs_a_session_on_the_system_clock() {
    let clock = SystemClock::new();
    let preset = PomodoroPreset::default();
    let mut engine = Engine::new();
    engine
        .start(preset.clone(), LockKind::UntilTimerEnds, &clock)
        .expect("a strict session starts on the system clock");

    let first = engine.status(&clock);
    assert_eq!(first.phase(), Phase::Focus);
    assert_eq!(first.cycle(), 1);
    assert!(first.remaining() <= preset.focus());
    assert!(
        first.remaining() + FAR_TOO_LONG > preset.focus(),
        "the session lost time right after it started: {:?} left",
        first.remaining()
    );

    thread::sleep(NAP);
    let later = engine.status(&clock);
    assert_eq!(later.phase(), Phase::Focus);
    assert!(
        later.remaining() + NAP <= first.remaining() + RESOLUTION_SLACK,
        "after {NAP:?} the remaining time went from {:?} to {:?}",
        first.remaining(),
        later.remaining()
    );
}

#[test]
fn system_clock_can_be_shared_as_a_clock_trait_object() {
    let shared: Arc<dyn Clock + Send + Sync> = Arc::new(SystemClock::new());
    let mut engine = Engine::new();
    engine
        .start(PomodoroPreset::default(), LockKind::None, shared.as_ref())
        .expect("a session starts on a shared clock");

    let from_thread = {
        let clock = Arc::clone(&shared);
        thread::spawn(move || read_many(clock.as_ref(), 1_000))
            .join()
            .expect("the reader thread failed")
    };

    assert_eq!(engine.status(shared.as_ref()).phase(), Phase::Focus);
    assert!(shared.monotonic() >= from_thread);
}
