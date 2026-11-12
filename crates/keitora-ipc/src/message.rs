// SPDX-License-Identifier: MIT OR Apache-2.0

//! The messages: requests, responses and events, and the envelope that wraps
//! each one.
//!
//! No request can end, shorten, pause (when strict), extend or re-time a
//! session, move a clock or edit the blocklist a strict session uses: the
//! only requests are the ones below, and the service judges each one
//! (`docs/threat-model.md`). No request carries a time, an instant or a
//! duration.

use keitora_core::engine::{LockKind, PomodoroPreset};
use serde::Serialize;

use crate::error::ProtocolError;
use crate::types::{
    BlockEvent, BlocklistSummary, HistoryEntry, HistoryRange, InstalledApp, MessageId, Outcome,
    PageRequest, SessionState, Settings, ValidatedBlocklist,
};

/// The version of the protocol this crate speaks, sent as `v` in every frame.
///
/// One whole number, the major version. A change that only adds (a message, a
/// field) keeps it; one that changes or removes something bumps it.
pub const PROTOCOL_VERSION: u32 = 1;

/// A message with the id that ties it to its request, or names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// The id of the message.
    id: MessageId,
    /// What it says.
    message: Message,
}

impl Envelope {
    /// A message at [`PROTOCOL_VERSION`] under `id`. A response carries the
    /// id of the request it answers; an event, an id of its own.
    #[must_use]
    pub fn new(id: MessageId, message: Message) -> Self {
        Self { id, message }
    }

    /// The id of the message.
    #[must_use]
    pub fn id(&self) -> &MessageId {
        &self.id
    }

    /// What the message says.
    #[must_use]
    pub fn message(&self) -> &Message {
        &self.message
    }
}

/// What an [`Envelope`] holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A client asks the service for something.
    Request(Request),
    /// The service answers a request.
    Response(Response),
    /// The service tells a subscribed client that something happened.
    Event(Event),
}

/// What a client can ask of the service. In `payload`, the `type` is the
/// variant's name in `snake_case`, and its fields sit next to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Request {
    /// The state of the session.
    GetState,
    /// Starts a session. The service can refuse it (`InvalidState` if one is
    /// running, `NotFound` for an unknown blocklist).
    StartSession {
        /// The blocklist to enforce.
        blocklist_id: String,
        /// The durations and cycles of the session.
        preset: PomodoroPreset,
        /// How firmly the session is locked.
        lock: LockKind,
    },
    /// Asks to stop the session. A strict one refuses; for a `RandomText`
    /// one, `text` is what the user typed (at most
    /// [`MAX_STOP_TEXT_BYTES`](crate::MAX_STOP_TEXT_BYTES) bytes).
    StopSession {
        /// The text typed to unlock a `RandomText` session.
        text: Option<String>,
    },
    /// Pauses the session; a strict one refuses.
    PauseSession,
    /// Resumes a paused session.
    ResumeSession,
    /// Ends the current break early.
    SkipBreak,
    /// A page of the blocklists.
    ListBlocklists {
        /// Which part of the list.
        page: PageRequest,
    },
    /// One blocklist, with its targets.
    GetBlocklist {
        /// The blocklist's id.
        id: String,
    },
    /// Creates a blocklist or replaces the one with the same id; a strict
    /// session refuses to change the blocklist it uses.
    UpsertBlocklist {
        /// The blocklist, already checked.
        blocklist: ValidatedBlocklist,
    },
    /// Deletes a blocklist; a strict session refuses to delete its own.
    DeleteBlocklist {
        /// The blocklist's id.
        id: String,
    },
    /// The user's settings.
    GetSettings,
    /// Replaces the user's settings.
    UpdateSettings {
        /// The new settings.
        settings: Settings,
    },
    /// A page of the applications the picker offers.
    ListInstalledApps {
        /// Which part of the list.
        page: PageRequest,
    },
    /// A page of the finished sessions.
    GetHistory {
        /// The span of time to look at.
        range: HistoryRange,
        /// Which part of the list.
        page: PageRequest,
    },
    /// A page of the sites and applications that were blocked.
    GetBlockEvents {
        /// Which part of the list.
        page: PageRequest,
    },
    /// Asks to receive the events of this connection.
    Subscribe,
}

/// What the service answers to a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Response {
    /// The request was done and has nothing to report.
    Ok,
    /// The connection is subscribed to events.
    Subscribed,
    /// The state of the session.
    State {
        /// The state.
        state: SessionState,
    },
    /// A page of blocklists.
    Blocklists {
        /// The blocklists of the page.
        items: Vec<BlocklistSummary>,
        /// Where the next page starts; `None` after the last.
        next_offset: Option<u32>,
    },
    /// One blocklist.
    Blocklist {
        /// The blocklist.
        blocklist: ValidatedBlocklist,
    },
    /// The user's settings.
    Settings {
        /// The settings.
        settings: Settings,
    },
    /// A page of applications.
    InstalledApps {
        /// The applications of the page.
        items: Vec<InstalledApp>,
        /// Where the next page starts; `None` after the last.
        next_offset: Option<u32>,
    },
    /// A page of the history.
    History {
        /// The sessions of the page.
        items: Vec<HistoryEntry>,
        /// Where the next page starts; `None` after the last.
        next_offset: Option<u32>,
    },
    /// A page of blocked sites and applications.
    BlockEvents {
        /// The events of the page.
        items: Vec<BlockEvent>,
        /// Where the next page starts; `None` after the last.
        next_offset: Option<u32>,
    },
    /// The request failed.
    Error {
        /// Why.
        error: ProtocolError,
    },
}

/// What the service tells subscribed clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Event {
    /// The session changed: a new phase, a pause, a resume, a start or an
    /// end.
    StateChanged {
        /// The new state.
        state: SessionState,
    },
    /// A site or an application was blocked.
    TargetBlocked {
        /// What was blocked.
        event: BlockEvent,
    },
    /// A blocklist was created, changed or deleted.
    BlocklistChanged {
        /// The blocklist's id.
        id: String,
    },
    /// The session ended.
    SessionEnded {
        /// How.
        outcome: Outcome,
    },
}
