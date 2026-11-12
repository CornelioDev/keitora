// SPDX-License-Identifier: MIT OR Apache-2.0

//! The typed errors of the protocol: what a service answers when it refuses a
//! request, and what a reader reports about a frame it can't use.
//!
//! An error never carries text the peer sent, nor anything about the
//! implementation (no parser messages, file names or positions): only a
//! stable code and the few fields named in each variant. See ADR-0014.

use keitora_core::engine::LockReason as CoreLockReason;
use keitora_core::model::ValidationErrorKind;
use serde::{Deserialize, Serialize};

use crate::message::{Envelope, Message, Response};
use crate::types::MessageId;

/// An error of the protocol.
///
/// On the wire it is an object tagged by `code`, such as
/// `{ "code": "validation", "field": "page.limit", "kind": "out_of_range" }`;
/// [`ProtocolError::code`] gives the code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProtocolError {
    /// A strict session is active, so the request is refused until it ends.
    #[error("a strict session is active")]
    StrictSessionLocked {
        /// Why the session can't be changed.
        reason: LockReason,
    },
    /// A value is outside its range or not in the expected form.
    #[error("a value is not valid: {field}")]
    Validation {
        /// The JSON path of the value, from the payload, such as
        /// `blocklist.targets[0].pattern`. Always chosen by this crate, never
        /// copied from the message.
        field: String,
        /// What is wrong with it.
        kind: ValidationKind,
    },
    /// The blocklist, or whatever else the request names, doesn't exist.
    #[error("not found")]
    NotFound,
    /// The message has a protocol version this peer doesn't speak.
    #[error("unsupported protocol version")]
    UnsupportedVersion {
        /// The versions this peer speaks.
        supported: Vec<u32>,
    },
    /// The frame is longer than [`MAX_FRAME_LEN`](crate::MAX_FRAME_LEN).
    #[error("frame too large (more than {max} bytes)")]
    FrameTooLarge {
        /// The most bytes a frame may have, not counting its newline.
        max: u32,
    },
    /// The frame isn't a message of this protocol: it isn't JSON, a field has
    /// the wrong type or is missing, or the message type or kind is unknown.
    #[error("malformed message")]
    Malformed,
    /// The request is well formed, but the engine refuses it in the state it
    /// is in: there is no session, one is already running, or it isn't
    /// paused.
    #[error("the request doesn't fit the current state")]
    InvalidState,
    /// The service failed in a way the client can't fix. The details stay in
    /// the service's log.
    #[error("internal error")]
    Internal,
}

impl ProtocolError {
    /// The stable name of this error on the wire (`"code"`).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::StrictSessionLocked { .. } => "strict_session_locked",
            Self::Validation { .. } => "validation",
            Self::NotFound => "not_found",
            Self::UnsupportedVersion { .. } => "unsupported_version",
            Self::FrameTooLarge { .. } => "frame_too_large",
            Self::Malformed => "malformed",
            Self::InvalidState => "invalid_state",
            Self::Internal => "internal",
        }
    }

    /// A validation error for the value at `field`.
    pub(crate) fn validation(field: impl Into<String>, kind: ValidationKind) -> Self {
        Self::Validation {
            field: field.into(),
            kind,
        }
    }
}

/// Why a strict session refuses a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockReason {
    /// The session is active until its timer ends.
    UntilTimerEnds,
}

impl From<CoreLockReason> for LockReason {
    fn from(reason: CoreLockReason) -> Self {
        match reason {
            CoreLockReason::UntilTimerEnds => Self::UntilTimerEnds,
            // `keitora_core::engine::LockReason` may grow. A reason this
            // crate doesn't know yet is still a lock, so it is reported as
            // the one that exists, never as no lock.
            _ => Self::UntilTimerEnds,
        }
    }
}

/// What is wrong with a value in a [`ProtocolError::Validation`].
///
/// It carries no numbers: the limits are part of the protocol's documentation
/// and of the model in `keitora-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ValidationKind {
    /// The text is empty.
    Empty,
    /// The text is longer than its maximum.
    TooLong,
    /// The list has more items than its maximum.
    TooMany,
    /// The text contains a control character.
    ControlCharacter,
    /// The id has a character other than an ASCII letter, a digit or `-`.
    NotAnId,
    /// The color isn't in the form `#rrggbb`.
    NotAColor,
    /// The executable name isn't a file name.
    NotAFileName,
    /// The path isn't a full path that starts with a drive.
    NotADrivePath,
    /// The application sets no identifier for any platform.
    NoIdentifier,
    /// The site pattern isn't one the matcher can use.
    NotASitePattern,
    /// The number is outside its range.
    OutOfRange,
    /// The value is invalid in a way none of the others names.
    Invalid,
}

impl From<ValidationErrorKind> for ValidationKind {
    fn from(kind: ValidationErrorKind) -> Self {
        match kind {
            ValidationErrorKind::Empty => Self::Empty,
            ValidationErrorKind::TooLong { .. } => Self::TooLong,
            ValidationErrorKind::TooMany { .. } => Self::TooMany,
            ValidationErrorKind::ControlCharacter => Self::ControlCharacter,
            ValidationErrorKind::NotAnId => Self::NotAnId,
            ValidationErrorKind::NotAColor => Self::NotAColor,
            ValidationErrorKind::NotAFileName => Self::NotAFileName,
            ValidationErrorKind::NotADrivePath => Self::NotADrivePath,
            ValidationErrorKind::NoIdentifier => Self::NoIdentifier,
            ValidationErrorKind::NotASitePattern => Self::NotASitePattern,
            // The core enum is non_exhaustive: a problem added there reaches
            // clients as a generic one until this crate names it.
            _ => Self::Invalid,
        }
    }
}

/// A frame that [`decode`](crate::decode) rejected, with what a receiver needs
/// to answer it.
///
/// Its `Display` and `Debug` output show the error and the id, never any
/// other part of the frame.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("rejected frame: {error}")]
pub struct DecodeError {
    /// Why the frame was rejected.
    error: ProtocolError,
    /// The id of the frame, if it had a valid one.
    id: Option<MessageId>,
}

impl DecodeError {
    /// A rejection with the id the frame had, if it was valid.
    pub(crate) fn new(error: ProtocolError, id: Option<MessageId>) -> Self {
        Self { error, id }
    }

    /// Why the frame was rejected.
    #[must_use]
    pub fn error(&self) -> &ProtocolError {
        &self.error
    }

    /// The id of the rejected frame, if it was read and was valid. An invalid
    /// id is never kept, so it can't be echoed back.
    #[must_use]
    pub fn id(&self) -> Option<&MessageId> {
        self.id.as_ref()
    }

    /// The error response to send back: under the frame's id, or under the
    /// reserved id `"unknown"` when it had none.
    #[must_use]
    pub fn into_response(self) -> Envelope {
        let id = self.id.unwrap_or_else(MessageId::unknown);
        Envelope::new(id, Message::Response(Response::Error { error: self.error }))
    }
}

#[cfg(test)]
mod tests;
