// SPDX-License-Identifier: MIT OR Apache-2.0

//! Entry point of the Keitora Windows service; the service itself is in the
//! `keitora_service` library.
//!
//! ```text
//! keitora-service                               service mode (started by the SCM)
//! keitora-service --console [--data-dir <DIR>]  console mode (development builds only)
//! ```

#![forbid(unsafe_code)]

use std::process::ExitCode;

use keitora_service::cli::{self, CONSOLE_MODE_AVAILABLE, Mode};
use keitora_service::{console, error_chain, print_error, service};

/// The exit code of a rejected command line.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    // First of all, before anything can load a DLL by name: the process runs
    // as SYSTEM, so DLLs come from System32 only, never from the folder of
    // the executable, the working folder or PATH (F0 security review, L10).
    // If it failed, both modes refuse to run, once they can say why: console
    // mode at once, on stderr; service mode once the SCM has connected and
    // logging is set up.
    let dll_search_restricted = keitora_sys::library_loader::restrict_dll_search_to_system32();

    let mode = match cli::parse(std::env::args_os().skip(1), CONSOLE_MODE_AVAILABLE) {
        Ok(mode) => mode,
        Err(error) => {
            print_error(&format!("{error}\n{}", cli::usage(CONSOLE_MODE_AVAILABLE)));
            return ExitCode::from(USAGE_ERROR);
        }
    };

    match mode {
        Mode::Console { data_dir } => console::run(data_dir, dll_search_restricted),
        Mode::Service => match service::run(dll_search_restricted) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                print_error(&format!(
                    "couldn't connect to the Service Control Manager: {}\n\
                     This program runs as the Keitora Windows service.{}",
                    error_chain(&error),
                    if CONSOLE_MODE_AVAILABLE {
                        " To run it in this terminal, use --console."
                    } else {
                        ""
                    }
                ));
                ExitCode::FAILURE
            }
        },
    }
}
