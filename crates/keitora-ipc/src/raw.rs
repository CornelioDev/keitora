// SPDX-License-Identifier: MIT OR Apache-2.0

//! Reading a payload: from its JSON text to a [`Request`], [`Response`] or
//! [`Event`].
//!
//! Serde's internally tagged enums and `flatten` copy every field of an object,
//! unknown ones included, into a buffer before they pick a variant, so a
//! client could make the service hold many times the size of a frame. Here
//! the `type` is read first by a struct that skips everything else, and then
//! the fields of that one message are read by a plain struct of its own,
//! which skips unknown fields without keeping them (as `keitora-core` does
//! for `Target`).
//!
//! A wrong structure or type is [`ProtocolError::Malformed`]; a value of the
//! right type but outside its range is [`ProtocolError::Validation`].

use keitora_core::engine::{LockKind, PomodoroPreset, PresetField};
use keitora_core::model::Blocklist;
use serde::Deserialize;

use crate::error::{LockReason, ProtocolError, ValidationKind};
use crate::message::{Event, Request, Response};
use crate::types::{
    BlockEvent, BlocklistSummary, HistoryEntry, HistoryRange, InstalledApp, MAX_STOP_TEXT_BYTES,
    Outcome, PageRequest, SessionState, Settings, ValidatedBlocklist, check_blocklist_id,
};

/// Reads `text` as a `T`; any failure is `Malformed`, and the parser's message
/// (which could quote the input) is dropped.
fn parse<'a, T: Deserialize<'a>>(text: &'a str) -> Result<T, ProtocolError> {
    serde_json::from_str(text).map_err(|_| ProtocolError::Malformed)
}

/// The `type` of a payload.
#[derive(Deserialize)]
struct Tag {
    #[serde(rename = "type")]
    name: String,
}

fn tag(text: &str) -> Result<String, ProtocolError> {
    parse::<Tag>(text).map(|tag| tag.name)
}

#[derive(Deserialize)]
struct RawPreset {
    focus_min: u32,
    short_break_min: u32,
    long_break_min: u32,
    cycles: u32,
}

/// Checks a preset; `path` is where it sits in the payload.
fn preset(raw: &RawPreset, path: &str) -> Result<PomodoroPreset, ProtocolError> {
    PomodoroPreset::new(
        raw.focus_min,
        raw.short_break_min,
        raw.long_break_min,
        raw.cycles,
    )
    .map_err(|error| {
        let name = match error.field() {
            PresetField::Focus => "focus_min",
            PresetField::ShortBreak => "short_break_min",
            PresetField::LongBreak => "long_break_min",
            PresetField::Cycles => "cycles",
        };
        ProtocolError::validation(format!("{path}.{name}"), ValidationKind::OutOfRange)
    })
}

#[derive(Deserialize)]
struct RawPage {
    offset: u32,
    limit: u32,
}

fn page(raw: &RawPage) -> Result<PageRequest, ProtocolError> {
    PageRequest::new(raw.offset, raw.limit)
}

#[derive(Deserialize)]
struct RawSettings {
    default_preset: RawPreset,
}

fn settings(raw: &RawSettings) -> Result<Settings, ProtocolError> {
    Ok(Settings {
        default_preset: preset(&raw.default_preset, "settings.default_preset")?,
    })
}

fn blocklist(raw: Blocklist) -> Result<ValidatedBlocklist, ProtocolError> {
    ValidatedBlocklist::new(raw).map_err(|error| {
        ProtocolError::validation(format!("blocklist.{}", error.field()), error.kind().into())
    })
}

/// Checks the id of a blocklist in a field named `field`.
fn blocklist_id(id: String, field: &str) -> Result<String, ProtocolError> {
    check_blocklist_id(&id)
        .map(|()| id)
        .map_err(|kind| ProtocolError::validation(field, kind))
}

#[derive(Deserialize)]
struct StartSession {
    blocklist_id: String,
    preset: RawPreset,
    lock: LockKind,
}

#[derive(Deserialize)]
struct StopSession {
    text: Option<String>,
}

#[derive(Deserialize)]
struct WithPage {
    page: RawPage,
}

#[derive(Deserialize)]
struct WithId {
    id: String,
}

#[derive(Deserialize)]
struct UpsertBlocklist {
    blocklist: Blocklist,
}

#[derive(Deserialize)]
struct UpdateSettings {
    settings: RawSettings,
}

#[derive(Deserialize)]
struct GetHistory {
    range: HistoryRange,
    page: RawPage,
}

/// Reads the payload of a request.
pub(crate) fn request(text: &str) -> Result<Request, ProtocolError> {
    Ok(match tag(text)?.as_str() {
        "get_state" => Request::GetState,
        "start_session" => {
            let raw: StartSession = parse(text)?;
            let blocklist_id = blocklist_id(raw.blocklist_id, "blocklist_id")?;
            let preset = preset(&raw.preset, "preset")?;
            raw.lock
                .validate()
                .map_err(|_| ProtocolError::validation("lock", ValidationKind::OutOfRange))?;
            Request::StartSession {
                blocklist_id,
                preset,
                lock: raw.lock,
            }
        }
        "stop_session" => {
            let raw: StopSession = parse(text)?;
            if raw
                .text
                .as_ref()
                .is_some_and(|t| t.len() > MAX_STOP_TEXT_BYTES)
            {
                return Err(ProtocolError::validation("text", ValidationKind::TooLong));
            }
            Request::StopSession { text: raw.text }
        }
        "pause_session" => Request::PauseSession,
        "resume_session" => Request::ResumeSession,
        "skip_break" => Request::SkipBreak,
        "list_blocklists" => Request::ListBlocklists {
            page: page(&parse::<WithPage>(text)?.page)?,
        },
        "get_blocklist" => Request::GetBlocklist {
            id: blocklist_id(parse::<WithId>(text)?.id, "id")?,
        },
        "upsert_blocklist" => Request::UpsertBlocklist {
            blocklist: blocklist(parse::<UpsertBlocklist>(text)?.blocklist)?,
        },
        "delete_blocklist" => Request::DeleteBlocklist {
            id: blocklist_id(parse::<WithId>(text)?.id, "id")?,
        },
        "get_settings" => Request::GetSettings,
        "update_settings" => Request::UpdateSettings {
            settings: settings(&parse::<UpdateSettings>(text)?.settings)?,
        },
        "list_installed_apps" => Request::ListInstalledApps {
            page: page(&parse::<WithPage>(text)?.page)?,
        },
        "get_history" => {
            let raw: GetHistory = parse(text)?;
            Request::GetHistory {
                range: raw.range,
                page: page(&raw.page)?,
            }
        }
        "get_block_events" => Request::GetBlockEvents {
            page: page(&parse::<WithPage>(text)?.page)?,
        },
        "subscribe" => Request::Subscribe,
        _ => return Err(ProtocolError::Malformed),
    })
}

#[derive(Deserialize)]
struct WithState {
    state: SessionState,
}

#[derive(Deserialize)]
struct WithSettings {
    settings: RawSettings,
}

#[derive(Deserialize)]
struct WithBlocklist {
    blocklist: Blocklist,
}

#[derive(Deserialize)]
struct Items<T> {
    items: Vec<T>,
    next_offset: Option<u32>,
}

#[derive(Deserialize)]
struct WithError {
    error: RawError,
}

/// Every field an error can have, as read.
#[derive(Deserialize)]
struct RawError {
    code: String,
    field: Option<String>,
    kind: Option<ValidationKind>,
    reason: Option<LockReason>,
    supported: Option<Vec<u32>>,
    max: Option<u32>,
}

fn protocol_error(raw: RawError) -> Result<ProtocolError, ProtocolError> {
    let malformed = || ProtocolError::Malformed;
    Ok(match raw.code.as_str() {
        "strict_session_locked" => ProtocolError::StrictSessionLocked {
            reason: raw.reason.ok_or_else(malformed)?,
        },
        "validation" => ProtocolError::Validation {
            field: raw.field.ok_or_else(malformed)?,
            kind: raw.kind.ok_or_else(malformed)?,
        },
        "not_found" => ProtocolError::NotFound,
        "unsupported_version" => ProtocolError::UnsupportedVersion {
            supported: raw.supported.ok_or_else(malformed)?,
        },
        "frame_too_large" => ProtocolError::FrameTooLarge {
            max: raw.max.ok_or_else(malformed)?,
        },
        "malformed" => ProtocolError::Malformed,
        "invalid_state" => ProtocolError::InvalidState,
        "internal" => ProtocolError::Internal,
        _ => return Err(ProtocolError::Malformed),
    })
}

/// Reads the payload of a response.
pub(crate) fn response(text: &str) -> Result<Response, ProtocolError> {
    Ok(match tag(text)?.as_str() {
        "ok" => Response::Ok,
        "subscribed" => Response::Subscribed,
        "state" => Response::State {
            state: parse::<WithState>(text)?.state,
        },
        "blocklists" => {
            let raw: Items<BlocklistSummary> = parse(text)?;
            Response::Blocklists {
                items: raw.items,
                next_offset: raw.next_offset,
            }
        }
        "blocklist" => Response::Blocklist {
            blocklist: blocklist(parse::<WithBlocklist>(text)?.blocklist)?,
        },
        "settings" => Response::Settings {
            settings: settings(&parse::<WithSettings>(text)?.settings)?,
        },
        "installed_apps" => {
            let raw: Items<InstalledApp> = parse(text)?;
            Response::InstalledApps {
                items: raw.items,
                next_offset: raw.next_offset,
            }
        }
        "history" => {
            let raw: Items<HistoryEntry> = parse(text)?;
            Response::History {
                items: raw.items,
                next_offset: raw.next_offset,
            }
        }
        "block_events" => {
            let raw: Items<BlockEvent> = parse(text)?;
            Response::BlockEvents {
                items: raw.items,
                next_offset: raw.next_offset,
            }
        }
        "error" => Response::Error {
            error: protocol_error(parse::<WithError>(text)?.error)?,
        },
        _ => return Err(ProtocolError::Malformed),
    })
}

#[derive(Deserialize)]
struct WithEvent {
    event: BlockEvent,
}

#[derive(Deserialize)]
struct WithOutcome {
    outcome: Outcome,
}

/// Reads the payload of an event.
pub(crate) fn event(text: &str) -> Result<Event, ProtocolError> {
    Ok(match tag(text)?.as_str() {
        "state_changed" => Event::StateChanged {
            state: parse::<WithState>(text)?.state,
        },
        "target_blocked" => Event::TargetBlocked {
            event: parse::<WithEvent>(text)?.event,
        },
        "blocklist_changed" => Event::BlocklistChanged {
            id: parse::<WithId>(text)?.id,
        },
        "session_ended" => Event::SessionEnded {
            outcome: parse::<WithOutcome>(text)?.outcome,
        },
        _ => return Err(ProtocolError::Malformed),
    })
}
