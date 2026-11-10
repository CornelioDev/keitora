// SPDX-License-Identifier: MIT OR Apache-2.0

//! Service mode: the binary started with no arguments, by the Service
//! Control Manager (SCM), through the `windows-service` crate.
//!
//! [`run`] hands the calling thread to the SCM's dispatcher. Outside the SCM,
//! from a terminal for example, connecting fails at once and [`run`] returns
//! an error, having written nothing. Under the SCM, the service:
//!
//! 1. sets up logging in `%ProgramData%\Keitora\logs` ([`crate::logging`]),
//!    the first time it writes anything;
//! 2. refuses to run if restricting the DLL search order to `System32`
//!    failed at the start of `main`: it logs why and reports itself stopped
//!    with an error, because a process running as `SYSTEM` must not load
//!    DLLs from folders a user could write to;
//! 3. reports itself running, accepting stop and shutdown requests;
//! 4. runs [`crate::runtime`] until one of those arrives, then reports itself
//!    stopped.

use std::ffi::OsString;
use std::io;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use crate::data_dir::{default_data_dir, logs_dir};
use crate::runtime::{self, StopReason};
use crate::{error_chain, logging};

/// The service's name in the SCM, which the installer registers it under.
pub const SERVICE_NAME: &str = "Keitora";

/// The exit code the service reports when it refuses to run, as a
/// service-specific error.
const REFUSED_TO_RUN: u32 = 1;

/// What restricting the DLL search order returned at the start of `main`.
/// The SCM calls [`service_main`] with no context of ours, so it is kept
/// here.
static DLL_SEARCH_RESTRICTED: OnceLock<io::Result<()>> = OnceLock::new();

// The entry point the SCM calls, `ffi_service_main`, which calls
// `service_main` with the service's arguments. The macro's expansion holds
// the one `unsafe` block of this crate's binary, written by `windows-service`
// to read those arguments; the lint that `#![forbid(unsafe_code)]` sets
// doesn't apply inside another crate's macro (ADR-0012).
define_windows_service!(ffi_service_main, service_main);

/// Hands the calling thread to the SCM, which runs the service on a thread
/// of its own, and returns when the service has stopped.
///
/// `dll_search_restricted` is what restricting the DLL search order to
/// `System32` returned at the start of `main`; if it failed, the service
/// refuses to run (see the [module documentation](self)).
///
/// # Errors
///
/// Returns the error of `StartServiceCtrlDispatcherW`, which fails at once
/// when the process wasn't started by the SCM.
pub fn run(dll_search_restricted: io::Result<()>) -> windows_service::Result<()> {
    // `run` is called once per process; were it called again, the first
    // result would stand.
    let _ = DLL_SEARCH_RESTRICTED.set(dll_search_restricted);
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

/// The service's main function, which the SCM calls on a thread of its own.
fn service_main(_arguments: Vec<OsString>) {
    // The arguments of a start request are ignored: whoever may start the
    // service doesn't get to configure it.
    match default_data_dir() {
        Ok(data_dir) => logging::init(&logs_dir(&data_dir)),
        Err(error) => logging::init_without_files(&error),
    }
    tracing::info!(
        "Keitora service {} starting as a Windows service",
        env!("CARGO_PKG_VERSION")
    );
    match run_service() {
        Ok(()) => tracing::info!("stopped"),
        Err(error) => tracing::error!(
            "the Service Control Manager rejected a request: {}",
            error_chain(&error)
        ),
    }
}

/// Registers for the SCM's requests, runs the service and reports its state.
fn run_service() -> windows_service::Result<()> {
    let (stop, stop_requests) = mpsc::channel();
    let status = service_control_handler::register(SERVICE_NAME, control_handler(stop))?;

    let refusal = match DLL_SEARCH_RESTRICTED.get() {
        Some(Ok(())) => None,
        Some(Err(error)) => Some(error_chain(error)),
        None => Some("the result was lost".to_owned()),
    };
    if let Some(reason) = refusal {
        tracing::error!("couldn't restrict where DLLs are loaded from, so it won't run: {reason}");
        status.set_service_status(service_status(
            ServiceState::Stopped,
            ServiceExitCode::ServiceSpecific(REFUSED_TO_RUN),
        ))?;
        return Ok(());
    }

    status.set_service_status(service_status(
        ServiceState::Running,
        ServiceExitCode::Win32(0),
    ))?;
    runtime::run(&stop_requests);
    status.set_service_status(service_status(
        ServiceState::Stopped,
        ServiceExitCode::Win32(0),
    ))
}

/// The handler of the SCM's requests: stop and shutdown send
/// [`StopReason::ServiceControl`] on `stop`.
fn control_handler(
    stop: Sender<StopReason>,
) -> impl Fn(ServiceControl) -> ServiceControlHandlerResult + Send + 'static {
    move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            // An error means the service is already stopping.
            let _ = stop.send(StopReason::ServiceControl);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    }
}

/// The status to report in `state`, with `exit_code`. A running service
/// accepts stop and shutdown requests; in any other state it accepts none.
fn service_status(state: ServiceState, exit_code: ServiceExitCode) -> ServiceStatus {
    let controls_accepted = if state == ServiceState::Running {
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
    } else {
        ServiceControlAccept::empty()
    };
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_and_shutdown_requests_stop_the_service() {
        for control in [ServiceControl::Stop, ServiceControl::Shutdown] {
            let (stop, stop_requests) = mpsc::channel();
            let handler = control_handler(stop);
            assert!(matches!(
                handler(control),
                ServiceControlHandlerResult::NoError
            ));
            assert_eq!(stop_requests.try_recv(), Ok(StopReason::ServiceControl));
        }
    }

    #[test]
    fn other_requests_dont_stop_the_service() {
        let (stop, stop_requests) = mpsc::channel();
        let handler = control_handler(stop);
        assert!(matches!(
            handler(ServiceControl::Interrogate),
            ServiceControlHandlerResult::NoError
        ));
        assert!(matches!(
            handler(ServiceControl::Pause),
            ServiceControlHandlerResult::NotImplemented
        ));
        assert!(stop_requests.try_recv().is_err());
    }

    #[test]
    fn only_a_running_service_accepts_requests() {
        let running = service_status(ServiceState::Running, ServiceExitCode::Win32(0));
        assert_eq!(
            running.controls_accepted,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        );
        let stopped = service_status(
            ServiceState::Stopped,
            ServiceExitCode::ServiceSpecific(REFUSED_TO_RUN),
        );
        assert!(stopped.controls_accepted.is_empty());
        assert_eq!(
            stopped.exit_code,
            ServiceExitCode::ServiceSpecific(REFUSED_TO_RUN)
        );
    }
}
