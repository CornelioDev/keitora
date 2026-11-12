// SPDX-License-Identifier: MIT OR Apache-2.0

//! Wall times as stored: whole milliseconds since the Unix epoch, in an
//! SQLite integer (`i64`), negative before 1970.
//!
//! The wall clock is the user's to set (ADR-0010), so any time Windows can
//! hold, 1601-01-01 to 30828, must convert without failing or panicking; at
//! about ±9.2 × 10¹⁸ ms, `i64` covers all of them. A time is rounded down to
//! its millisecond (towards the past, also before 1970), so that the order
//! of times is kept.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Nanoseconds in a millisecond.
const NANOS_PER_MILLI: u32 = 1_000_000;

/// `time` in whole milliseconds since the Unix epoch, rounded down, or
/// `None` if that doesn't fit in an `i64` (never for a time Windows can
/// hold).
pub(super) fn to_millis(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => {
            let before = before.duration();
            let mut millis = before.as_millis();
            // Rounding down before the epoch means away from it.
            if before.subsec_nanos().checked_rem(NANOS_PER_MILLI) != Some(0) {
                millis = millis.checked_add(1)?;
            }
            i64::try_from(millis).ok()?.checked_neg()
        }
    }
}

/// Like [`to_millis`], but a time too far from the epoch becomes the
/// smallest or largest `i64`: for the bounds of a range, where a time
/// beyond every stored one means "no bound".
pub(super) fn to_millis_saturating(time: SystemTime) -> i64 {
    to_millis(time).unwrap_or(if time < UNIX_EPOCH {
        i64::MIN
    } else {
        i64::MAX
    })
}

/// The wall time `millis` milliseconds after the Unix epoch (before it if
/// negative), or `None` if the platform can't represent it.
pub(super) fn from_millis(millis: i64) -> Option<SystemTime> {
    let offset = Duration::from_millis(millis.unsigned_abs());
    if millis >= 0 {
        UNIX_EPOCH.checked_add(offset)
    } else {
        UNIX_EPOCH.checked_sub(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> SystemTime {
        from_millis(ms).expect("representable")
    }

    #[test]
    fn whole_milliseconds_round_trip_on_both_sides_of_the_epoch() {
        // 1601-01-01, the epoch, and the last millisecond of 9999.
        for ms in [-11_644_473_600_000, -1, 0, 1, 253_402_300_799_999] {
            assert_eq!(to_millis(at(ms)), Some(ms));
        }
    }

    #[test]
    fn a_time_is_rounded_down_to_its_millisecond() {
        let after = UNIX_EPOCH + Duration::new(1, 999_900);
        assert_eq!(to_millis(after), Some(1_000));
        // 100 ns, the resolution of `SystemTime` on Windows.
        let before = UNIX_EPOCH - Duration::new(1, 100);
        assert_eq!(to_millis(before), Some(-1_001));
        let just_before = UNIX_EPOCH - Duration::from_nanos(100);
        assert_eq!(to_millis(just_before), Some(-1));
    }

    #[test]
    fn range_bounds_saturate_instead_of_failing() {
        assert_eq!(to_millis_saturating(at(5)), 5);
        // `SystemTime` on Windows can't reach past i64 milliseconds, so the
        // saturation itself is only reachable on other platforms; check
        // that it never panics at the extremes Windows has.
        let earliest = UNIX_EPOCH - Duration::from_secs(11_644_473_600);
        assert_eq!(to_millis_saturating(earliest), -11_644_473_600_000);
    }

    #[test]
    fn a_stored_time_beyond_the_platform_is_none_not_a_panic() {
        // On Windows, `SystemTime` starts in 1601.
        if cfg!(windows) {
            assert_eq!(from_millis(i64::MIN), None);
        }
        let _ = from_millis(i64::MAX);
    }
}
