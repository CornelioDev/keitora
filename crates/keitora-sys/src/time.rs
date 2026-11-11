// SPDX-License-Identifier: MIT OR Apache-2.0

//! Reading Windows' clocks.

use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;

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
    fn unbiased_interrupt_time_advances_with_real_time() {
        let start = unbiased_interrupt_time().expect("the first reading succeeds");
        std::thread::sleep(std::time::Duration::from_millis(100));
        let end = unbiased_interrupt_time().expect("the second reading succeeds");
        // 100 ms is 1,000,000 units; allow for the 15.6 ms timer step.
        assert!(end - start >= 800_000, "moved only {} units", end - start);
    }
}
