// SPDX-License-Identifier: MIT OR Apache-2.0

//! The Keitora Windows service, the single source of truth for sessions and
//! blocklists. It will persist state, block applications and serve the
//! desktop app and the native messaging host over a named pipe
//! (`docs/architecture.md`).
//!
//! The `keitora-service` binary is a thin entry point: it parses its command
//! line with [`cli`] and runs one of two modes, which share [`runtime`]:
//!
//! - [`service`]: started by the Service Control Manager (SCM), as installed;
//! - [`console`]: in the foreground of a terminal, for development builds only.
//!
//! The service keeps its files in a data folder ([`data_dir`]) and logs to
//! daily files inside it ([`logging`]). Time comes from [`clock::SystemClock`].
//!
//! This crate contains no `unsafe` code: Win32 calls go through the safe
//! wrappers of `keitora-sys` (ADR-0012).

#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::Write as _;
use std::io::{self, Write as _};

pub mod cli;
pub mod clock;
pub mod console;
pub mod data_dir;
pub mod logging;
pub mod runtime;
pub mod service;

/// The executable's name, which starts every message it prints on stderr.
pub const PROGRAM_NAME: &str = "keitora-service";

/// `error` followed by each of its sources, joined by `": "`.
///
/// Many errors keep the detail in their source, such as the Win32 error
/// behind an "IO error in winapi call"; printing only `error` would lose it.
/// A source whose message already ends the text, because the error before it
/// prints it too, isn't repeated.
#[must_use]
pub fn error_chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.ends_with(&cause_text) {
            // Writing to a `String` can't fail.
            let _ = write!(text, ": {cause_text}");
        }
        source = cause.source();
    }
    text
}

/// Prints `message` on stderr, as one line that starts with
/// [`PROGRAM_NAME`].
///
/// Unlike `eprintln!`, this never panics: if stderr is closed or missing,
/// as it is in a service, the message is dropped.
pub fn print_error(message: &str) {
    let _ = writeln!(io::stderr().lock(), "{PROGRAM_NAME}: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] io::Error);

    #[derive(Debug, thiserror::Error)]
    #[error("outer: {0}")]
    struct OuterThatPrintsItsSource(#[source] io::Error);

    #[test]
    fn error_chain_lists_every_source() {
        let error = Outer(io::Error::other("inner"));
        assert_eq!(error_chain(&error), "outer: inner");
        assert_eq!(error_chain(&io::Error::other("alone")), "alone");
    }

    #[test]
    fn error_chain_doesnt_repeat_a_source_the_error_already_prints() {
        let error = OuterThatPrintsItsSource(io::Error::other("inner"));
        assert_eq!(error_chain(&error), "outer: inner");
    }
}
