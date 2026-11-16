// SPDX-License-Identifier: MIT OR Apache-2.0

//! Reading Windows' clocks and time zone.

use windows_sys::Win32::System::Time::{
    GetTimeZoneInformation, TIME_ZONE_ID_INVALID, TIME_ZONE_INFORMATION,
};
use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;

/// What `GetTimeZoneInformation` returns while standard time is in effect
/// (`TIME_ZONE_ID_STANDARD`, defined in a much larger API family).
const TIME_ZONE_ID_STANDARD: u32 = 1;

/// What `GetTimeZoneInformation` returns while daylight saving time is in
/// effect (`TIME_ZONE_ID_DAYLIGHT`).
const TIME_ZONE_ID_DAYLIGHT: u32 = 2;

/// The number of 100-nanosecond units the system has been awake since it
/// started, from `QueryUnbiasedInterruptTime`.
///
/// It is the system's interrupt time without the time spent asleep or
/// hibernated, so it:
///
/// - never goes backwards and isn't affected by changes to the system time;
/// - stops while the machine sleeps or hibernates, and starts again from 0
///   after a restart;
/// - is 64 bits wide: it would take about 58,000 years awake to wrap, unlike
///   the 32-bit `GetTickCount`, which wraps after 49.7 days;
/// - moves in steps of the system timer, usually 15.6 ms.
///
/// Returns `None` if Windows reports a failure, which its documentation
/// limits to being passed a null pointer, and this function never passes
/// one.
#[must_use]
pub fn unbiased_interrupt_time() -> Option<u64> {
    let mut time: u64 = 0;
    // SAFETY: `&raw mut time` points to a live, aligned `u64` owned by this
    // function, which is what the function writes its result to, and nothing
    // else reads or writes `time` during the call.
    let succeeded = unsafe { QueryUnbiasedInterruptTime(&raw mut time) };
    (succeeded != 0).then_some(time)
}

/// How many minutes local time is ahead of UTC right now (negative west of
/// Greenwich), from `GetTimeZoneInformation`, with daylight saving time
/// applied when it is in effect.
///
/// It is the offset of the system's time zone, which only an administrator
/// can change, at the instant of the call: near a change to or from daylight
/// saving time, it may differ from the offset a few hours earlier or later.
///
/// Returns `None` if Windows reports a failure.
#[must_use]
pub fn local_utc_offset_minutes() -> Option<i32> {
    let mut zone = TIME_ZONE_INFORMATION::default();
    // SAFETY: `&raw mut zone` points to a live, aligned
    // `TIME_ZONE_INFORMATION` owned by this function, which is the structure
    // the function fills in, and nothing else reads or writes `zone` during
    // the call.
    let current = unsafe { GetTimeZoneInformation(&raw mut zone) };
    // Windows defines UTC = local time + bias, in minutes.
    let bias = match current {
        TIME_ZONE_ID_INVALID => return None,
        TIME_ZONE_ID_STANDARD => zone.Bias.checked_add(zone.StandardBias)?,
        TIME_ZONE_ID_DAYLIGHT => zone.Bias.checked_add(zone.DaylightBias)?,
        // `TIME_ZONE_ID_UNKNOWN`: the zone has no daylight saving time.
        _ => zone.Bias,
    };
    bias.checked_neg()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbiased_interrupt_time_is_read_and_never_goes_backwards() {
        let mut previous = unbiased_interrupt_time().expect("the first reading succeeds");
        for _ in 0..10_000 {
            let current = unbiased_interrupt_time().expect("a reading succeeds");
            assert!(current >= previous, "{previous} then {current}");
            previous = current;
        }
    }

    #[test]
    fn the_local_offset_is_read_and_is_a_real_one() {
        let offset = local_utc_offset_minutes().expect("the time zone is read");
        // Real offsets run from UTC-12:00 to UTC+14:00.
        assert!((-12 * 60..=14 * 60).contains(&offset), "{offset} minutes");
    }

    #[test]
    fn unbiased_interrupt_time_advances_with_real_time() {
        let start = unbiased_interrupt_time().expect("the first reading succeeds");
        std::thread::sleep(std::time::Duration::from_millis(100));
        let end = unbiased_interrupt_time().expect("the second reading succeeds");
        // 100 ms is 1,000,000 units; allow for the 15.6 ms timer step.
        assert!(end - start >= 800_000, "moved only {} units", end - start);
    }
}
