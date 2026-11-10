// SPDX-License-Identifier: MIT OR Apache-2.0

//! The service's real clock, [`SystemClock`].
//!
//! # Where its time comes from
//!
//! - The monotonic reading is Windows' *unbiased interrupt time*
//!   (`QueryUnbiasedInterruptTime`, through `keitora-sys`): how long the
//!   machine has been awake since it started, in 100 ns units. It is 64 bits
//!   wide, so it doesn't wrap (the 32-bit `GetTickCount` wraps after 49.7
//!   days, and is never used), and it ignores changes to the system time.
//! - The wall reading is [`SystemTime::now`], the calendar time the user can
//!   change. Nothing that enforces a session relies on it.
//!
//! # Sleep and hibernation don't count
//!
//! The unbiased interrupt time stops while the machine sleeps or hibernates.
//! So time asleep or hibernated **doesn't count toward a session's active
//! time** ([ADR-0010]): a focus phase with 10 minutes left before the lid is
//! closed still has 10 minutes left when the machine wakes up, strict or not.
//! This matches what happens when the machine is off, where the time between
//! the last snapshot and the restart isn't credited either ([ADR-0011]);
//! whether to credit either of them is open in
//! [#19](https://github.com/CornelioDev/keitora/issues/19). `Instant` was not
//! used because the standard library doesn't specify whether it counts time
//! asleep.
//!
//! # One clock for the whole service
//!
//! The service creates one `SystemClock` when it starts and shares it, by
//! reference or in an `Arc`, with everything that needs time, the engine
//! first. No client chooses, moves or replaces it: no IPC message, argument
//! or setting reaches it. Tests use `keitora_core::clock::FakeClock`, which
//! this crate's `clippy.toml` bans outside them.
//!
//! [ADR-0010]: https://github.com/CornelioDev/keitora/blob/main/docs/adr/0010-session-time-as-active-time.md
//! [ADR-0011]: https://github.com/CornelioDev/keitora/blob/main/docs/adr/0011-validated-engine-snapshot.md

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use keitora_core::clock::{Clock, MonotonicInstant};

/// Nanoseconds in one unit of the interrupt time.
const NANOS_PER_TICK: u64 = 100;

/// The service's [`Clock`]: a monotonic reading from Windows' unbiased
/// interrupt time and a wall reading from the system time (see the [module
/// documentation](self)).
///
/// The monotonic reading's origin is when the machine started, not counting
/// time asleep. It never goes backwards, also when several threads share one
/// clock: each reading is at least the latest one this clock has returned.
#[derive(Debug, Default)]
pub struct SystemClock {
    /// The latest interrupt time this clock has returned, in 100 ns units.
    latest: AtomicU64,
}

impl SystemClock {
    /// A clock that reads the system's time.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            latest: AtomicU64::new(0),
        }
    }

    /// `reading` if it is later than every reading returned so far, or the
    /// latest of those; `None`, a failed reading, also gives the latest.
    /// Either way, it becomes the latest.
    fn never_backwards(&self, reading: Option<u64>) -> u64 {
        let reading = reading.unwrap_or(0);
        // A read-modify-write on one atomic always sees the latest value
        // written to it, whatever the ordering, so `Relaxed` is enough to
        // never return less than an earlier reading.
        let previous = self.latest.fetch_max(reading, Ordering::Relaxed);
        previous.max(reading)
    }
}

impl Clock for SystemClock {
    fn monotonic(&self) -> MonotonicInstant {
        let ticks = self.never_backwards(keitora_sys::time::unbiased_interrupt_time());
        MonotonicInstant::from_duration_since_origin(ticks_to_duration(ticks))
    }

    fn wall(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// The duration of `ticks` 100 ns units. It saturates after about 584 years
/// awake, where it stands still instead of wrapping.
fn ticks_to_duration(ticks: u64) -> Duration {
    Duration::from_nanos(ticks.saturating_mul(NANOS_PER_TICK))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_hundreds_of_nanoseconds() {
        assert_eq!(ticks_to_duration(0), Duration::ZERO);
        assert_eq!(ticks_to_duration(1), Duration::from_nanos(100));
        assert_eq!(ticks_to_duration(10_000_000), Duration::from_secs(1));
    }

    #[test]
    fn ticks_saturate_instead_of_wrapping() {
        assert_eq!(ticks_to_duration(u64::MAX), Duration::from_nanos(u64::MAX));
    }

    #[test]
    fn a_reading_earlier_than_the_latest_returns_the_latest() {
        let clock = SystemClock::new();
        assert_eq!(clock.never_backwards(Some(500)), 500);
        assert_eq!(clock.never_backwards(Some(400)), 500);
        assert_eq!(clock.never_backwards(Some(600)), 600);
    }

    #[test]
    fn a_failed_reading_stands_still() {
        let clock = SystemClock::new();
        assert_eq!(clock.never_backwards(None), 0);
        assert_eq!(clock.never_backwards(Some(700)), 700);
        assert_eq!(clock.never_backwards(None), 700);
    }

    #[test]
    fn the_monotonic_reading_is_the_interrupt_time() {
        let clock = SystemClock::new();
        let before = keitora_sys::time::unbiased_interrupt_time().expect("a reading");
        let reading = clock.monotonic();
        let after = keitora_sys::time::unbiased_interrupt_time().expect("a reading");
        assert!(reading >= MonotonicInstant::from_duration_since_origin(ticks_to_duration(before)));
        assert!(reading <= MonotonicInstant::from_duration_since_origin(ticks_to_duration(after)));
    }
}
