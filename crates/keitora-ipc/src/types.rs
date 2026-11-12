// SPDX-License-Identifier: MIT OR Apache-2.0

//! The values that messages carry: ids, pages, validated blocklists and the
//! plain data the service reports.
//!
//! Everything a client can send is checked when it is built or decoded, so
//! the types here can't hold an invalid blocklist, a page without a cap or an
//! id the service wouldn't accept.

use keitora_core::engine::{LockKind, Phase as CorePhase, PomodoroPreset};
use keitora_core::model::{Blocklist, MAX_ID_LEN, ValidationError};
use serde::{Deserialize, Serialize};

use crate::error::{ProtocolError, ValidationKind};

/// The most items a page may ask for.
pub const MAX_PAGE_LIMIT: u32 = 100;

/// The most bytes of text a stop request may carry: the text the user types
/// to stop a `RandomText` session, which is at most 256 characters long
/// ([`MAX_RANDOM_TEXT_LEN`](keitora_core::engine::MAX_RANDOM_TEXT_LEN)), with
/// room for multi-byte ones.
pub const MAX_STOP_TEXT_BYTES: usize = 1024;

/// The longest message id, in bytes.
const MAX_MESSAGE_ID_LEN: usize = 64;

/// The id that answers a frame whose own id was missing or invalid.
const UNKNOWN_ID: &str = "unknown";

/// Whether `text` is an id in the form of a blocklist's: ASCII letters,
/// digits and `-`. Reports the same kinds of problem as `keitora-core`'s check
/// for the id of a blocklist, in the same order.
pub(crate) fn check_id(text: &str, max: usize) -> Result<(), ValidationKind> {
    if text.is_empty() {
        Err(ValidationKind::Empty)
    } else if text.len() > max {
        Err(ValidationKind::TooLong)
    } else if text.chars().any(char::is_control) {
        Err(ValidationKind::ControlCharacter)
    } else if text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        Ok(())
    } else {
        Err(ValidationKind::NotAnId)
    }
}

/// Checks the id of a blocklist named in a request, such as `blocklist_id`.
pub(crate) fn check_blocklist_id(text: &str) -> Result<(), ValidationKind> {
    check_id(text, MAX_ID_LEN)
}

/// The id that ties a response to its request.
///
/// 1 to 64 bytes: ASCII letters, digits and `-` (a UUID fits). The limit
/// keeps ids short enough to log and keeps anything but plain text out of
/// them, as the service puts an id in the answer to a bad frame.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct MessageId(String);

impl MessageId {
    /// A message id.
    ///
    /// # Errors
    ///
    /// [`ProtocolError::Validation`] for the field `id` if the text is empty,
    /// longer than 64 bytes or has a character outside the set above.
    pub fn new(text: &str) -> Result<Self, ProtocolError> {
        check_id(text, MAX_MESSAGE_ID_LEN)
            .map(|()| Self(text.to_owned()))
            .map_err(|kind| ProtocolError::validation("id", kind))
    }

    /// The reserved id of an answer to a frame that had no usable id.
    pub(crate) fn unknown() -> Self {
        Self(UNKNOWN_ID.to_owned())
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which part of a list a request asks for. The limit is capped, so no
/// request can ask for an unbounded list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct PageRequest {
    /// How many items to skip.
    offset: u32,
    /// How many items to return, from 1 to [`MAX_PAGE_LIMIT`].
    limit: u32,
}

impl PageRequest {
    /// A page of at most `limit` items, after skipping `offset`.
    ///
    /// # Errors
    ///
    /// [`ProtocolError::Validation`] for the field `page.limit` if `limit` is
    /// 0 or more than [`MAX_PAGE_LIMIT`].
    pub fn new(offset: u32, limit: u32) -> Result<Self, ProtocolError> {
        if (1..=MAX_PAGE_LIMIT).contains(&limit) {
            Ok(Self { offset, limit })
        } else {
            Err(ProtocolError::validation(
                "page.limit",
                ValidationKind::OutOfRange,
            ))
        }
    }

    /// How many items to skip.
    #[must_use]
    pub fn offset(&self) -> u32 {
        self.offset
    }

    /// How many items to return.
    #[must_use]
    pub fn limit(&self) -> u32 {
        self.limit
    }
}

/// A blocklist that passed [`Blocklist::validate`].
///
/// It is the only way to put a blocklist in a message, so an invalid one
/// can't be sent, and can't be decoded either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ValidatedBlocklist(Blocklist);

impl ValidatedBlocklist {
    /// Checks a blocklist.
    ///
    /// # Errors
    ///
    /// The first problem [`Blocklist::validate`] finds, with the path of the
    /// value that has it.
    pub fn new(blocklist: Blocklist) -> Result<Self, ValidationError> {
        blocklist.validate()?;
        Ok(Self(blocklist))
    }

    /// The blocklist.
    #[must_use]
    pub fn as_blocklist(&self) -> &Blocklist {
        &self.0
    }

    /// The blocklist, by value.
    #[must_use]
    pub fn into_blocklist(self) -> Blocklist {
        self.0
    }
}

/// A phase of a session, or [`Phase::Idle`] when there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// No session is running.
    Idle,
    /// Working; the blocklist is enforced.
    Focus,
    /// The break after every focus but the last.
    ShortBreak,
    /// The break after the last focus.
    LongBreak,
}

impl From<CorePhase> for Phase {
    fn from(phase: CorePhase) -> Self {
        match phase {
            CorePhase::Idle => Self::Idle,
            CorePhase::Focus => Self::Focus,
            CorePhase::ShortBreak => Self::ShortBreak,
            CorePhase::LongBreak => Self::LongBreak,
        }
    }
}

/// What kind of thing a blocklist target is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// A website.
    Site,
    /// An application.
    App,
}

/// How a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Every phase ran to its end.
    Completed,
    /// The user stopped it early.
    Stopped,
}

/// The span of history a request asks for. Always a name, never dates: no
/// request carries a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryRange {
    /// Since midnight, local time.
    Today,
    /// The last seven days.
    Week,
    /// The last thirty days.
    Month,
    /// Everything kept.
    All,
}

/// What the service reports about the session, as of when it answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    /// The phase the session is in.
    pub phase: Phase,
    /// The number of the focus phase the session is in, from 1.
    pub cycle: u32,
    /// Time left in the phase, in milliseconds.
    pub remaining_ms: u64,
    /// Whether the session is paused.
    pub paused: bool,
    /// How the session is locked; `None` when there is no session.
    pub lock: Option<LockKind>,
    /// The blocklist the session uses; `None` when there is no session.
    pub blocklist_id: Option<String>,
}

/// The user's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Settings {
    /// The preset a new session starts with.
    pub default_preset: PomodoroPreset,
}

/// A blocklist as listed, without its targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlocklistSummary {
    /// The blocklist's id.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// The color shown next to the name, as `#rrggbb`.
    pub color: String,
}

/// An application the picker offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledApp {
    /// The name shown to the user.
    pub name: String,
    /// The executable's file name, if it is known.
    pub windows_exe: Option<String>,
}

/// A finished session, as listed in the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// When the session started, in milliseconds since the Unix epoch.
    pub started_at_unix_ms: u64,
    /// The blocklist it used.
    pub blocklist_id: String,
    /// The preset it ran with.
    pub preset: PomodoroPreset,
    /// How it was locked.
    pub lock: LockKind,
    /// How it ended.
    pub outcome: Outcome,
    /// Time spent in focus phases, in milliseconds.
    pub focus_ms: u64,
}

/// A site or an application that was blocked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEvent {
    /// When it happened, in milliseconds since the Unix epoch.
    pub at_unix_ms: u64,
    /// Whether it was a site or an application.
    pub kind: TargetKind,
    /// The pattern or the name of the program that was blocked.
    pub target: String,
}

#[cfg(test)]
mod tests;
