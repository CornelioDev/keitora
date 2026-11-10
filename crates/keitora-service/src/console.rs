// SPDX-License-Identifier: MIT OR Apache-2.0

//! Console mode, for development: the service runs in the foreground of a
//! terminal, with its files in `--data-dir` or `%ProgramData%\Keitora`.
//!
//! It stops cleanly, with exit code 0, when:
//!
//! - its standard input reaches its end (Ctrl+Z then Enter in a terminal, or
//!   a parent process closing the pipe), or can't be read;
//! - the user presses Ctrl+C or Ctrl+Break.
//!
//! Only builds with debug assertions have it ([`crate::cli`]).

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::data_dir::{default_data_dir, logs_dir};
use crate::runtime::{self, StopReason};
use crate::{error_chain, logging, print_error};

/// Runs the service in console mode until it is told to stop.
///
/// `data_dir` is the folder given with `--data-dir`, if any.
/// `dll_search_restricted` is what restricting the DLL search order to
/// `System32` returned at the start of `main`; if it failed, console mode
/// refuses to run, as service mode does, and writes nothing.
#[must_use]
pub fn run(data_dir: Option<PathBuf>, dll_search_restricted: io::Result<()>) -> ExitCode {
    if let Err(error) = dll_search_restricted {
        print_error(&format!(
            "couldn't restrict where DLLs are loaded from, so it won't run: {}",
            error_chain(&error)
        ));
        return ExitCode::FAILURE;
    }

    match data_dir.map_or_else(default_data_dir, Ok) {
        Ok(data_dir) => logging::init(&logs_dir(&data_dir)),
        Err(error) => logging::init_without_files(&error),
    }
    tracing::info!(
        "Keitora service {} starting in console mode",
        env!("CARGO_PKG_VERSION")
    );

    let (stop, stop_requests) = mpsc::channel();
    stop_when_stdin_closes(stop.clone());
    stop_on_stop_keys(stop);
    runtime::run(&stop_requests);

    tracing::info!("stopped");
    ExitCode::SUCCESS
}

/// Sends [`StopReason::StdinClosed`] on `stop` when standard input reaches
/// its end or fails, from a thread of its own. What is read is discarded.
fn stop_when_stdin_closes(stop: Sender<StopReason>) {
    let spawned = thread::Builder::new()
        .name("stdin watcher".to_owned())
        .spawn(move || {
            // Returns at the end of the input, or on a read error.
            let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
            let _ = stop.send(StopReason::StdinClosed);
        });
    if let Err(error) = spawned {
        tracing::warn!(
            "can't watch standard input, so closing it won't stop the service: {}",
            error_chain(&error)
        );
    }
}

/// Sends [`StopReason::Key`] on `stop` when the user presses Ctrl+C or
/// Ctrl+Break.
fn stop_on_stop_keys(stop: Sender<StopReason>) {
    let registered = keitora_sys::console::on_stop_key(move |key| {
        let _ = stop.send(StopReason::Key(key));
    });
    if let Err(error) = registered {
        tracing::warn!(
            "can't handle Ctrl+C and Ctrl+Break, so they will end the service at once: {}",
            error_chain(&error)
        );
    }
}
