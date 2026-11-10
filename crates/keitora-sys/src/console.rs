// SPDX-License-Identifier: MIT OR Apache-2.0

//! Console control events: Ctrl+C and Ctrl+Break.

use std::fmt;
use std::io;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows_sys::core::BOOL;

/// A console key combination that asks the process to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopKey {
    /// Ctrl+C.
    CtrlC,
    /// Ctrl+Break.
    CtrlBreak,
}

impl fmt::Display for StopKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CtrlC => "Ctrl+C",
            Self::CtrlBreak => "Ctrl+Break",
        })
    }
}

/// What [`on_stop_key`] calls; set once per process.
type Callback = Box<dyn Fn(StopKey) + Send + Sync>;

/// The callback of [`on_stop_key`]. Windows calls [`handler_routine`] with no
/// context of ours, so the callback has to be reachable from a static.
static CALLBACK: OnceLock<Callback> = OnceLock::new();

/// Calls `callback` when the user presses Ctrl+C or Ctrl+Break in the
/// process's console, instead of letting Windows end the process at once.
///
/// Windows calls `callback` on a thread of its own, so it should only signal
/// another thread, for example by sending on a channel, and return quickly.
/// It must not panic: a panic there ends the process. Other console events,
/// such as closing the console window, keep their default behavior.
///
/// Only one callback can be set per process; this is meant for a program's
/// `main`.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::AlreadyExists`] if a callback was
/// already set, or the Win32 error if `SetConsoleCtrlHandler` fails. Either
/// way, Ctrl+C and Ctrl+Break keep ending the process.
pub fn on_stop_key<F>(callback: F) -> io::Result<()>
where
    F: Fn(StopKey) + Send + Sync + 'static,
{
    CALLBACK.set(Box::new(callback)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a console stop key callback is already set",
        )
    })?;
    // SAFETY: `handler_routine` has the signature `PHANDLER_ROUTINE` asks for
    // and is a plain function, so it stays valid for the life of the process;
    // it never unwinds, as it only reads a static and calls a callback whose
    // contract forbids panicking (a panic would abort, not unwind, across the
    // `extern "system"` boundary).
    let succeeded = unsafe { SetConsoleCtrlHandler(Some(handler_routine), TRUE) };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The `HandlerRoutine` Windows calls on a console control event. Returns
/// `TRUE` when the event was handled, and `FALSE` to pass it on to the next
/// handler, which for the events not handled here is Windows' default one.
extern "system" fn handler_routine(control_type: u32) -> BOOL {
    let key = match control_type {
        CTRL_C_EVENT => StopKey::CtrlC,
        CTRL_BREAK_EVENT => StopKey::CtrlBreak,
        _ => return FALSE,
    };
    match CALLBACK.get() {
        Some(callback) => {
            callback(key);
            TRUE
        }
        None => FALSE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `on_stop_key` isn't called here: it would change how the test process
    // reacts to Ctrl+C, and it can only be called once per process.

    #[test]
    fn events_other_than_the_stop_keys_are_passed_on() {
        // CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT and CTRL_SHUTDOWN_EVENT.
        for control_type in [2, 5, 6] {
            assert_eq!(handler_routine(control_type), FALSE);
        }
    }

    #[test]
    fn stop_keys_are_named_for_people() {
        assert_eq!(StopKey::CtrlC.to_string(), "Ctrl+C");
        assert_eq!(StopKey::CtrlBreak.to_string(), "Ctrl+Break");
    }
}
