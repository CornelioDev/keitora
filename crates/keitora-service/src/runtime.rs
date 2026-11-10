// SPDX-License-Identifier: MIT OR Apache-2.0

//! What the service does once it has started, the same in service and
//! console mode: for now, wait until it is told to stop. The engine, the
//! pipe server and the process monitor join it in later tasks (T-2.4 to
//! T-2.6).

use std::fmt;
use std::sync::mpsc::Receiver;

use keitora_sys::console::StopKey;

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

/// Runs the service until a [`StopReason`] arrives on `stop`, and returns it.
///
/// If every sender of `stop` is dropped, it returns
/// [`StopReason::NoStopSource`] rather than run with no way to stop.
pub fn run(stop: &Receiver<StopReason>) -> StopReason {
    tracing::info!("running");
    let reason = stop.recv().unwrap_or(StopReason::NoStopSource);
    tracing::info!("stopping: {reason}");
    reason
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
        assert_eq!(run(&receiver), StopReason::StdinClosed);
    }

    #[test]
    fn run_returns_when_nothing_can_stop_it_any_more() {
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        assert_eq!(run(&receiver), StopReason::NoStopSource);
    }

    #[test]
    fn stop_reasons_read_as_sentences() {
        assert_eq!(
            StopReason::Key(StopKey::CtrlBreak).to_string(),
            "Ctrl+Break was pressed"
        );
    }
}
