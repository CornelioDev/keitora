// SPDX-License-Identifier: MIT OR Apache-2.0

//! The protocol spoken between the Keitora service and its clients (the
//! desktop app and the native messaging host): message types, versioning and
//! line-delimited JSON framing.
//!
//! Like `keitora-core`, this crate has no operating-system dependencies, so
//! every client can share it and it can be tested anywhere. The protocol is
//! described in `docs/architecture.md`, and its decisions in ADR-0014.
//!
//! A frame is one line of compact JSON ending in `\n`, at most
//! [`MAX_FRAME_LEN`] bytes long:
//! `{ "v": 1, "id": "...", "kind": "request", "payload": { "type": "get_state" } }`.
//! [`FrameReader`] cuts a byte stream into frames, [`decode`] checks one and
//! [`encode`] writes one. Everything a client sends is validated while it is
//! decoded, and errors are typed ([`ProtocolError`]) and never echo the input.
//!
//! # Example
//!
//! ```
//! use keitora_ipc::{Envelope, FrameReader, Message, MessageId, Request, decode, encode};
//!
//! // A client writes a request...
//! let request = Envelope::new(MessageId::new("req-1")?, Message::Request(Request::GetState));
//! let bytes = encode(&request)?;
//!
//! // ...and the service cuts its input into frames and decodes each one.
//! let mut reader = FrameReader::new();
//! for frame in reader.push(&bytes) {
//!     let received = decode(&frame?)?;
//!     assert_eq!(received, request);
//! }
//!
//! // A frame it can't read is answered with a typed error and the stream goes on.
//! let rejected = decode(b"not json").unwrap_err();
//! let answer = keitora_ipc::encode(&rejected.into_response())?;
//! assert!(answer.starts_with(b"{"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

mod codec;
mod error;
mod frame;
mod message;
mod raw;
mod types;

pub use codec::{decode, encode};
pub use error::{DecodeError, LockReason, ProtocolError, ValidationKind};
pub use frame::{FrameReader, MAX_FRAME_LEN};
pub use message::{Envelope, Event, Message, PROTOCOL_VERSION, Request, Response};
pub use types::{
    BlockEvent, BlocklistSummary, HistoryEntry, HistoryRange, InstalledApp, MAX_PAGE_LIMIT,
    MAX_STOP_TEXT_BYTES, MessageId, Outcome, PageRequest, Phase, SessionState, Settings,
    TargetKind, ValidatedBlocklist,
};

#[cfg(test)]
mod tests;
