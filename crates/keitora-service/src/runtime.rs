// SPDX-License-Identifier: MIT OR Apache-2.0

//! What the service does once it has started, the same in service and
//! console mode:
//!
//! 1. It opens the store in the data folder and creates the
//!    [`ServiceEngine`] with the service's one [`SystemClock`], the only
//!    clock it ever creates.
//! 2. It serves the engine on the pipe [`PIPE_NAME`]. If the pipe can't be
//!    created, for instance because another process serves its name, it
//!    keeps running and tries again every [`PIPE_RETRY`]: what the engine
//!    enforces must never depend on the pipe (ADR-0015).
//! 3. It calls [`ServiceEngine::tick`] every [`TICK_INTERVAL`] until it is
//!    told to stop; a tick that panics is logged, and the next one runs.
//! 4. When told to stop, it closes the pipe and saves a last checkpoint.
//!
//! If the store can't be opened, or its snapshot can't be read, the service
//! runs without its engine and without the pipe until it is stopped, and
//! logs why. That keeps a stored session from being overwritten, but
//! enforces nothing: T-2.7 revisits it (ADR-0017). The process monitor joins in T-2.6.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use keitora_core::clock::Clock;
use keitora_ipc::Event;
use keitora_sys::console::StopKey;

use crate::clock::SystemClock;
use crate::engine::{self, EngineStartError, EventSink, ServiceEngine};
use crate::error_chain;
use crate::pipe::{PIPE_NAME, PipeServer, RequestHandler};
use crate::store::Store;

/// How often the engine is ticked.
pub const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// How long the service waits before trying again to create a pipe that it
/// couldn't. A whole number of [`TICK_INTERVAL`]s.
pub const PIPE_RETRY: Duration = Duration::from_secs(5);

/// Why the service stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The Service Control Manager sent a stop or shutdown request.
    ServiceControl,
    /// Console mode: standard input reached its end or couldn't be read.
    StdinClosed,
    /// Console mode: the user pressed a stop key.
    Key(StopKey),
    /// Every sender of stop requests is gone, so none can come any more.
    NoStopSource,
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceControl => f.write_str("the Service Control Manager asked it to"),
            Self::StdinClosed => f.write_str("standard input was closed"),
            Self::Key(key) => write!(f, "{key} was pressed"),
            Self::NoStopSource => f.write_str("nothing can ask it to stop any more"),
        }
    }
}

/// Runs the service, with its files in `data_dir`, until a [`StopReason`]
/// arrives on `stop`, and returns it.
///
/// With no `data_dir`, as when `%ProgramData%` isn't set, it runs without
/// the engine (see the [module documentation](self)). If every sender of
/// `stop` is dropped, it returns [`StopReason::NoStopSource`] rather than
/// run with no way to stop.
pub fn run(stop: &Receiver<StopReason>, data_dir: Option<&Path>) -> StopReason {
    tracing::info!("running");
    let started = match data_dir {
        Some(data_dir) => start_engine(data_dir),
        None => {
            tracing::error!("no data folder, so the service runs without its engine");
            None
        }
    };
    let reason = match started {
        Some((engine, events)) => serve(stop, &engine, &events),
        None => stop.recv().unwrap_or(StopReason::NoStopSource),
    };
    tracing::info!("stopping: {reason}");
    reason
}

/// Opens the store in `data_dir` and creates the engine on it, with the
/// service's clock and an event sink for the pipe. Logs and returns `None`
/// if either fails.
fn start_engine(data_dir: &Path) -> Option<(Arc<ServiceEngine>, Arc<PipeEvents>)> {
    let store = match Store::open(data_dir) {
        Ok(store) => store,
        Err(error) => {
            tracing::error!(
                "couldn't open the database, so the service runs without its engine: {}",
                engine::describe(&error)
            );
            return None;
        }
    };
    // The service's one clock: no client chooses, moves or replaces it.
    let clock: Arc<dyn Clock + Send + Sync> = Arc::new(SystemClock::new());
    let events = Arc::new(PipeEvents::default());
    let sink: Arc<dyn EventSink> = Arc::<PipeEvents>::clone(&events);
    match ServiceEngine::new(store, clock, sink) {
        Ok(engine) => Some((Arc::new(engine), events)),
        Err(EngineStartError::LoadSnapshot(error)) => {
            tracing::error!(
                "couldn't read the stored engine snapshot, so the service runs without its \
                 engine: {}",
                engine::describe(&error)
            );
            None
        }
    }
}

/// Serves `engine` on the pipe and ticks it until a stop request arrives.
fn serve(
    stop: &Receiver<StopReason>,
    engine: &Arc<ServiceEngine>,
    events: &PipeEvents,
) -> StopReason {
    let retry_ticks = PIPE_RETRY
        .as_secs()
        .checked_div(TICK_INTERVAL.as_secs())
        .unwrap_or(1)
        .max(1);
    let mut pipe_failures: u32 = 0;
    let mut ticks_to_retry: u64 = 0;
    let reason = loop {
        if !events.is_serving() {
            if ticks_to_retry == 0 {
                if events.serve(engine, pipe_failures) {
                    pipe_failures = 0;
                } else {
                    pipe_failures = pipe_failures.saturating_add(1);
                    ticks_to_retry = retry_ticks;
                }
            } else {
                ticks_to_retry = ticks_to_retry.saturating_sub(1);
            }
        }
        if catch_unwind(AssertUnwindSafe(|| engine.tick())).is_err() {
            tracing::error!("a tick of the engine panicked; the next one runs as usual");
        }
        match stop.recv_timeout(TICK_INTERVAL) {
            Ok(reason) => break reason,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break StopReason::NoStopSource,
        }
    };
    events.stop_serving();
    if catch_unwind(AssertUnwindSafe(|| engine.checkpoint())).is_err() {
        tracing::error!("the last checkpoint panicked");
    }
    reason
}

/// Sends the engine's events to the subscribers of the pipe server, while
/// one runs; events published while none does reach no one.
#[derive(Debug, Default)]
struct PipeEvents {
    /// The running pipe server, if any.
    server: Mutex<Option<PipeServer>>,
}

impl PipeEvents {
    /// The server, behind its lock. A panic while it was held changes
    /// nothing a later call relies on.
    fn server(&self) -> MutexGuard<'_, Option<PipeServer>> {
        self.server.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a pipe server runs.
    fn is_serving(&self) -> bool {
        self.server().is_some()
    }

    /// Starts serving `engine` on [`PIPE_NAME`]. Returns whether it could.
    /// `failures` is how many attempts in a row failed before, so that only
    /// the first failure of a row is logged as a warning.
    fn serve(&self, engine: &Arc<ServiceEngine>, failures: u32) -> bool {
        let handler: Arc<dyn RequestHandler> = Arc::<ServiceEngine>::clone(engine);
        match PipeServer::start(PIPE_NAME, handler) {
            Ok(server) => {
                *self.server() = Some(server);
                tracing::info!("serving clients on the pipe");
                true
            }
            Err(error) if failures == 0 => {
                tracing::warn!(
                    "couldn't serve the pipe, trying again every {} s: {}",
                    PIPE_RETRY.as_secs(),
                    error_chain(&error)
                );
                false
            }
            Err(error) => {
                tracing::debug!("still couldn't serve the pipe: {}", error_chain(&error));
                false
            }
        }
    }

    /// Stops the pipe server, if one runs.
    fn stop_serving(&self) {
        // Taken out first, so that the lock isn't held while the server
        // waits for the requests in progress, which may publish events.
        let server = self.server().take();
        if let Some(server) = server {
            server.shutdown();
        }
    }
}

impl EventSink for PipeEvents {
    fn publish(&self, event: Event) {
        if let Some(server) = &*self.server() {
            server.publish(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn run_returns_the_first_stop_reason() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(StopReason::StdinClosed)
            .expect("the receiver is alive");
        sender
            .send(StopReason::ServiceControl)
            .expect("the receiver is alive");
        assert_eq!(run(&receiver, None), StopReason::StdinClosed);
    }

    #[test]
    fn run_returns_when_nothing_can_stop_it_any_more() {
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        assert_eq!(run(&receiver, None), StopReason::NoStopSource);
    }

    #[test]
    fn stop_reasons_read_as_sentences() {
        assert_eq!(
            StopReason::Key(StopKey::CtrlBreak).to_string(),
            "Ctrl+Break was pressed"
        );
    }

    #[test]
    fn events_with_no_server_go_nowhere() {
        let events = PipeEvents::default();
        assert!(!events.is_serving());
        events.publish(Event::BlocklistChanged {
            id: "work".to_owned(),
        });
        events.stop_serving();
        assert!(!events.is_serving());
    }

    #[test]
    fn the_pipe_is_retried_after_whole_ticks() {
        assert!(PIPE_RETRY >= TICK_INTERVAL);
        assert_eq!(PIPE_RETRY.subsec_nanos(), 0);
        assert_eq!(TICK_INTERVAL.subsec_nanos(), 0);
    }
}
