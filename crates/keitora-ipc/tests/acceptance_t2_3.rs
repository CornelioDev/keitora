// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.3: the IPC protocol in `keitora-ipc`: versioned
//! requests, responses, events and typed errors, with size limits.
//!
//! The task's own criterion is "serialization tests and version-compatibility
//! tests". The task also carries items from earlier security reviews, which
//! are tested here too. Nothing in this file may be changed except by QA.
//!
//! # Criteria covered
//!
//! Serialization (AC-S):
//!
//! - S1 every request, response and event has a golden JSON frame in
//!   `tests/acceptance/wire_v1.json`, which is part of the contract:
//!   `every_message_encodes_to_its_golden_frame`,
//!   `every_golden_frame_decodes_to_its_message`,
//!   `the_fixture_has_exactly_the_sample_messages`,
//!   `frame_too_large_error_has_its_wire_shape`.
//! - S2 a frame is one line of JSON ending in `\n`, with the envelope of
//!   `docs/architecture.md`: `every_encoded_frame_is_one_line_with_the_envelope`,
//!   `events_carry_the_protocol_version`.
//! - S3 names on the wire are stable: `enum_wire_names_are_stable`,
//!   `error_codes_are_stable`, `phase_names_decode_to_phases`.
//! - S4 round trip of anything decoded: `whatever_decodes_re_encodes_to_the_same_message`.
//!
//! Version compatibility (AC-V):
//!
//! - V1 a newer major version is rejected with the typed error, keeping the
//!   request id, before the payload is looked at:
//!   `a_newer_version_is_rejected_with_unsupported_version`,
//!   `the_version_is_checked_before_the_payload`.
//! - V2 a missing or non-integer version is malformed:
//!   `a_missing_or_non_integer_version_is_malformed`.
//! - V3 additive changes keep the version: unknown fields are ignored, at
//!   every level, and optional fields may be absent or null:
//!   `unknown_fields_are_ignored_at_every_level`,
//!   `optional_fields_may_be_absent_or_null`.
//! - V4 an unknown message type is a typed `malformed` error that keeps the id
//!   and does not break the stream: `an_unknown_message_type_is_malformed_and_keeps_the_id`,
//!   `an_unknown_event_does_not_break_the_stream`,
//!   `a_kind_that_does_not_match_the_payload_is_malformed`.
//!
//! Validation at decode (AC-D):
//!
//! - D1 an invalid blocklist is rejected with a typed validation error and
//!   never reaches the service: `an_invalid_blocklist_is_rejected_at_decode`,
//!   `validated_blocklist_only_holds_valid_blocklists`,
//!   `a_blocklist_at_every_model_maximum_is_accepted`.
//! - D2 locks, presets, ids, pages and typed text are bounded:
//!   `a_random_text_lock_out_of_range_is_rejected`,
//!   `presets_out_of_range_are_rejected`, `blocklist_ids_are_validated`,
//!   `page_limits_are_capped`, `stop_text_is_capped`,
//!   `message_ids_are_validated`,
//!   `a_lock_with_inconsistent_fields_is_malformed`,
//!   `validation_kinds_map_from_the_core`.
//!
//! Security by construction (AC-X):
//!
//! - X1 no request stops, cancels, pauses, shortens or re-times a session
//!   other than the listed ones, and a stop carries the typed text:
//!   `there_is_no_request_that_bypasses_the_session_rules`,
//!   `time_fields_in_a_request_are_ignored`.
//! - X2 typed errors never echo client text nor leak internals:
//!   `errors_never_echo_client_text`, `error_responses_carry_nothing_else`.
//! - X3 the protocol offers no way to read or move a clock, and has no
//!   message that carries a URL to check yet:
//!   `there_is_no_request_that_bypasses_the_session_rules`.
//!
//! Limits and framing (AC-F):
//!
//! - F1 the size limit counts raw bytes before parsing, with the boundary at
//!   exactly `MAX_FRAME_LEN`: `a_frame_over_the_limit_is_rejected_before_it_is_parsed`,
//!   `a_frame_of_exactly_the_limit_is_accepted`.
//! - F2 the limit is consistent with the model's maxima:
//!   `the_frame_limit_covers_the_largest_valid_blocklist`.
//! - F3 oversized frames are rejected without being buffered, and the reader
//!   recovers at the next newline: `an_oversized_frame_is_refused_without_buffering_it`,
//!   `the_reader_recovers_after_an_oversized_frame`,
//!   `an_oversized_frame_ending_in_the_same_chunk_is_reported_once`.
//! - F4 partial frames, several frames in one buffer, empty frames, NULs and
//!   invalid UTF-8 are handled without panics: `the_reader_keeps_partial_frames`,
//!   `the_reader_splits_several_frames_in_one_buffer`,
//!   `the_reader_gives_the_same_frames_for_any_chunking`,
//!   `the_reader_skips_empty_frames`,
//!   `the_reader_passes_nuls_and_invalid_utf8_to_the_decoder`,
//!   `malformed_input_is_a_typed_error_never_a_panic`,
//!   `every_truncated_prefix_of_a_frame_is_malformed`,
//!   `random_input_never_panics`.
//! - F5 the encoder enforces the same limit: `encoding_a_frame_over_the_limit_fails`.
//! - F6 lists are paginated with a capped page: see D2 (`page_limits_are_capped`)
//!   and the `page`/`next_offset` fields in the golden frames.
//!
//! Housekeeping items of the task (AC-H):
//!
//! - H1 `disallowed-methods` in the three `clippy.toml` files:
//!   `split_off_sum_and_product_are_banned_in_all_three_clippy_files`,
//!   `bytes_methods_are_banned_if_the_crate_is_used`.
//! - H2 `keitora-ipc` stays pure: `the_crate_stays_pure`.
//! - H3 a crate-level usage example and no placeholder test:
//!   `the_crate_docs_have_a_usage_example`,
//!   `the_placeholder_test_is_gone`, `the_crate_left_the_placeholder_list`.
//!
//! # Expected public API (everything at the crate root of `keitora_ipc`)
//!
//! Constants: `PROTOCOL_VERSION: u32` (= 1); `MAX_FRAME_LEN: usize`, the most
//! bytes in a frame, not counting the `\n`; `MAX_PAGE_LIMIT: u32` (= 100);
//! `MAX_STOP_TEXT_BYTES: usize` (= 1024).
//!
//! Functions:
//!
//! - `encode(&Envelope) -> Result<Vec<u8>, ProtocolError>`: one line of
//!   compact JSON ending in `\n`; `FrameTooLarge` if it would pass the limit.
//! - `decode(&[u8]) -> Result<Envelope, DecodeError>`: takes a frame *without*
//!   its `\n`. The length is checked first, on the raw bytes.
//!
//! Types (all `Debug`; `Clone`, `PartialEq` and `Eq` unless a field prevents
//! it; the enums' variants are plain, so tests build them with struct
//! syntax, and none of them is `#[non_exhaustive]` at variant level):
//!
//! - `MessageId`: `new(&str) -> Result<MessageId, ProtocolError>` (1 to 64
//!   bytes, ASCII letters, digits and `-`), `as_str()`.
//! - `Envelope`: `new(MessageId, Message)`, `id() -> &MessageId`,
//!   `message() -> &Message`. The version is always `PROTOCOL_VERSION`.
//! - `Message`: `Request(Request)`, `Response(Response)`, `Event(Event)`.
//! - `DecodeError`: `std::error::Error`; `error() -> &ProtocolError`,
//!   `id() -> Option<&MessageId>` (the id of the rejected frame when it had a
//!   valid one), `into_response(self) -> Envelope`, an error response to send
//!   back, with the frame's id or `"unknown"` when it has none.
//! - `FrameReader`: `new()`, `Default`, `push(&mut self, &[u8]) ->
//!   Vec<Result<Vec<u8>, ProtocolError>>` returning, in order, each complete
//!   frame (without its `\n`) or the error for one over the limit;
//!   `buffered_len() -> usize`.
//! - `ValidatedBlocklist`: `new(keitora_core::model::Blocklist) ->
//!   Result<Self, keitora_core::model::ValidationError>`, `as_blocklist()`,
//!   `into_blocklist()`. It is the only way to put a blocklist in a message.
//! - `PageRequest`: `new(offset: u32, limit: u32) -> Result<Self,
//!   ProtocolError>` (limit 1 to `MAX_PAGE_LIMIT`), `offset()`, `limit()`.
//! - `Request`, `Response`, `Event`, `ProtocolError`: below.
//! - Plain data with public fields: `SessionState`, `Settings`,
//!   `BlocklistSummary`, `InstalledApp`, `HistoryEntry`, `BlockEvent`.
//! - Plain enums: `Phase` (`Idle`, `Focus`, `ShortBreak`, `LongBreak`, with
//!   `From<keitora_core::engine::Phase>`), `TargetKind` (`Site`, `App`),
//!   `Outcome` (`Completed`, `Stopped`), `HistoryRange` (`Today`, `Week`,
//!   `Month`, `All`), `LockReason` (`UntilTimerEnds`, with
//!   `From<keitora_core::engine::LockReason>`), `ValidationKind`
//!   (`Empty`, `TooLong`, `TooMany`, `ControlCharacter`, `NotAnId`,
//!   `NotAColor`, `NotAFileName`, `NotADrivePath`, `NoIdentifier`,
//!   `NotASitePattern`, `OutOfRange`, `Invalid`, with
//!   `From<keitora_core::model::ValidationErrorKind>`).
//!
//! `Request` variants: `GetState`; `StartSession { blocklist_id: String,
//! preset: PomodoroPreset, lock: LockKind }`; `StopSession { text:
//! Option<String> }`; `PauseSession`; `ResumeSession`; `SkipBreak`;
//! `ListBlocklists { page }`; `GetBlocklist { id }`; `UpsertBlocklist {
//! blocklist: ValidatedBlocklist }`; `DeleteBlocklist { id }`; `GetSettings`;
//! `UpdateSettings { settings: Settings }`; `ListInstalledApps { page }`;
//! `GetHistory { range: HistoryRange, page }`; `GetBlockEvents { page }`;
//! `Subscribe`. (`page` is a `PageRequest`.)
//!
//! `Response` variants: `Ok`; `Subscribed`; `State { state: SessionState }`;
//! `Blocklists { items: Vec<BlocklistSummary>, next_offset: Option<u32> }`;
//! `Blocklist { blocklist: ValidatedBlocklist }`; `Settings { settings }`;
//! `InstalledApps { items: Vec<InstalledApp>, next_offset }`; `History {
//! items: Vec<HistoryEntry>, next_offset }`; `BlockEvents { items:
//! Vec<BlockEvent>, next_offset }`; `Error { error: ProtocolError }`.
//!
//! `Event` variants: `StateChanged { state: SessionState }`; `TargetBlocked {
//! event: BlockEvent }`; `BlocklistChanged { id: String }`; `SessionEnded {
//! outcome: Outcome }`.
//!
//! `ProtocolError` variants, with `code() -> &'static str` in brackets:
//! `StrictSessionLocked { reason: LockReason }` [`strict_session_locked`];
//! `Validation { field: String, kind: ValidationKind }` [`validation`];
//! `NotFound` [`not_found`]; `UnsupportedVersion { supported: Vec<u32> }`
//! [`unsupported_version`]; `FrameTooLarge { max: u32 }`
//! [`frame_too_large`]; `Malformed` [`malformed`]; `InvalidState`
//! [`invalid_state`], for a request the engine refuses in this state (no
//! session, already running, not paused); `Internal` [`internal`]. It is a
//! `std::error::Error`.
//!
//! # Wire format (chosen here; the golden file is the full contract)
//!
//! - Transport framing: line-delimited JSON, as `docs/architecture.md` says.
//! - Envelope: `{ "v": 1, "id": "<id>", "kind": "request" | "response" |
//!   "event", "payload": { "type": "<snake_case name>", ...fields } }`. The
//!   payload is flat, with its fields next to `type`. Serde's internally
//!   tagged enums and `flatten` buffer unknown fields (a 25x memory
//!   amplification found in T-1.1), so the implementation reads the `type`
//!   with a flat struct instead, as `keitora-core` does for `Target`.
//! - Encoding writes every field, `null` for an empty option. Decoding accepts
//!   an absent field as `null` for options.
//! - Errors: `{ "code": "<code>", ...fields }` inside `{ "type": "error",
//!   "error": ... }`. Fields are only the ones named above.
//! - Times that the service reports are `*_unix_ms` (history, block events)
//!   and `remaining_ms`; no request holds a time, an instant or a duration,
//!   and `GetHistory` takes a named range, never dates.
//!
//! # Decisions
//!
//! - **Version:** one integer, the major version. A non-negative integer other
//!   than 1 is `UnsupportedVersion { supported: [1] }`; anything else is
//!   `Malformed`. Additive changes keep `v`; a breaking change bumps it.
//! - **Unknown fields are ignored**, in the envelope, the payload and nested
//!   objects (as `keitora-core` already does). An unknown `type` or `kind` is
//!   `Malformed`: the receiver answers or skips it and goes on.
//! - **Validation:** structure and types wrong is `Malformed`; a well-formed
//!   value outside its range is `Validation { field, kind }`, with the JSON
//!   path of the field from the payload (`blocklist.targets[0].pattern`,
//!   `preset.focus_min`, `lock`, `blocklist_id`, `id`, `text`,
//!   `page.limit`, `settings.default_preset.cycles`).
//! - **Wrong `Stop` text, no session, and the like** are the service's
//!   business (T-2.5); the protocol only gives it `StopSession { text }`.

#![cfg(test)]

use std::collections::BTreeSet;

use keitora_core::engine::{
    LockKind, LockReason as CoreLockReason, MAX_CYCLES, MAX_FOCUS_MIN, MAX_LONG_BREAK_MIN,
    MAX_RANDOM_TEXT_LEN, MAX_SHORT_BREAK_MIN, Phase as CorePhase, PomodoroPreset,
};
use keitora_core::model::{
    AppIdentifiers, Blocklist, MAX_APP_ID_LEN, MAX_EXCEPTIONS, MAX_ID_LEN, MAX_NAME_LEN,
    MAX_PATTERN_LEN, MAX_TARGETS, SitePattern, Target, ValidationErrorKind,
};
use keitora_ipc::{
    BlockEvent, BlocklistSummary, DecodeError, Envelope, Event, FrameReader, HistoryEntry,
    HistoryRange, InstalledApp, LockReason, MAX_FRAME_LEN, MAX_PAGE_LIMIT, MAX_STOP_TEXT_BYTES,
    Message, MessageId, Outcome, PROTOCOL_VERSION, PageRequest, Phase, ProtocolError, Request,
    Response, SessionState, Settings, TargetKind, ValidatedBlocklist, ValidationKind, decode,
    encode,
};
use serde_json::{Value, json};

const ROOT_CLIPPY: &str = include_str!("../../../clippy.toml");
const CORE_CLIPPY: &str = include_str!("../../keitora-core/clippy.toml");
const IPC_CLIPPY: &str = include_str!("../clippy.toml");
const IPC_CARGO_TOML: &str = include_str!("../Cargo.toml");
const IPC_LIB_RS: &str = include_str!("../src/lib.rs");
const LAYOUT_RULES: &str = include_str!("../../../scripts/unit-test-layout-rules.mjs");
const GOLDEN: &str = include_str!("acceptance/wire_v1.json");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mid(text: &str) -> MessageId {
    MessageId::new(text).expect("a valid message id")
}

fn page(offset: u32, limit: u32) -> PageRequest {
    PageRequest::new(offset, limit).expect("a valid page")
}

fn preset() -> PomodoroPreset {
    PomodoroPreset::default()
}

fn blocklist() -> Blocklist {
    Blocklist {
        id: "work".to_owned(),
        name: "Work".to_owned(),
        color: "#3366ff".to_owned(),
        targets: vec![
            Target::Site {
                pattern: SitePattern::new("youtube.com"),
            },
            Target::App {
                name: "Steam".to_owned(),
                ids: AppIdentifiers {
                    windows_exe: Some("steam.exe".to_owned()),
                    ..AppIdentifiers::default()
                },
            },
        ],
        exceptions: vec![SitePattern::new("music.youtube.com")],
        block_during_breaks: false,
    }
}

fn validated() -> ValidatedBlocklist {
    ValidatedBlocklist::new(blocklist()).expect("the sample blocklist is valid")
}

fn state() -> SessionState {
    SessionState {
        phase: Phase::Focus,
        cycle: 2,
        remaining_ms: 1_234_567,
        paused: false,
        lock: Some(LockKind::UntilTimerEnds),
        blocklist_id: Some("work".to_owned()),
    }
}

fn sample(name: &'static str, message: Message) -> (&'static str, Envelope) {
    let id = format!("t-{}", name.replace('_', "-"));
    (name, Envelope::new(mid(&id), message))
}

/// Every message of the protocol, by the name it has in the golden file.
fn samples() -> Vec<(&'static str, Envelope)> {
    let req = |name: &'static str, request: Request| sample(name, Message::Request(request));
    let res = |name: &'static str, response: Response| sample(name, Message::Response(response));
    let evt = |name: &'static str, event: Event| sample(name, Message::Event(event));
    let error = |name: &'static str, error: ProtocolError| res(name, Response::Error { error });
    vec![
        req("req_get_state", Request::GetState),
        req(
            "req_start_session",
            Request::StartSession {
                blocklist_id: "work".to_owned(),
                preset: preset(),
                lock: LockKind::UntilTimerEnds,
            },
        ),
        req(
            "req_stop_session_without_text",
            Request::StopSession { text: None },
        ),
        req(
            "req_stop_session_with_text",
            Request::StopSession {
                text: Some("k3Zq9x".to_owned()),
            },
        ),
        req("req_pause_session", Request::PauseSession),
        req("req_resume_session", Request::ResumeSession),
        req("req_skip_break", Request::SkipBreak),
        req(
            "req_list_blocklists",
            Request::ListBlocklists { page: page(0, 50) },
        ),
        req(
            "req_get_blocklist",
            Request::GetBlocklist {
                id: "work".to_owned(),
            },
        ),
        req(
            "req_upsert_blocklist",
            Request::UpsertBlocklist {
                blocklist: validated(),
            },
        ),
        req(
            "req_delete_blocklist",
            Request::DeleteBlocklist {
                id: "work".to_owned(),
            },
        ),
        req("req_get_settings", Request::GetSettings),
        req(
            "req_update_settings",
            Request::UpdateSettings {
                settings: Settings {
                    default_preset: preset(),
                },
            },
        ),
        req(
            "req_list_installed_apps",
            Request::ListInstalledApps { page: page(20, 20) },
        ),
        req(
            "req_get_history",
            Request::GetHistory {
                range: HistoryRange::Week,
                page: page(20, 20),
            },
        ),
        req(
            "req_get_block_events",
            Request::GetBlockEvents { page: page(20, 20) },
        ),
        req("req_subscribe", Request::Subscribe),
        res("res_ok", Response::Ok),
        res("res_subscribed", Response::Subscribed),
        res("res_state", Response::State { state: state() }),
        res(
            "res_blocklists",
            Response::Blocklists {
                items: vec![BlocklistSummary {
                    id: "work".to_owned(),
                    name: "Work".to_owned(),
                    color: "#3366ff".to_owned(),
                }],
                next_offset: None,
            },
        ),
        res(
            "res_blocklist",
            Response::Blocklist {
                blocklist: validated(),
            },
        ),
        res(
            "res_settings",
            Response::Settings {
                settings: Settings {
                    default_preset: preset(),
                },
            },
        ),
        res(
            "res_installed_apps",
            Response::InstalledApps {
                items: vec![InstalledApp {
                    name: "Steam".to_owned(),
                    windows_exe: Some("steam.exe".to_owned()),
                }],
                next_offset: Some(20),
            },
        ),
        res(
            "res_history",
            Response::History {
                items: vec![HistoryEntry {
                    started_at_unix_ms: 1_700_000_000_000,
                    blocklist_id: "work".to_owned(),
                    preset: preset(),
                    lock: LockKind::None,
                    outcome: Outcome::Completed,
                    focus_ms: 6_000_000,
                }],
                next_offset: None,
            },
        ),
        res(
            "res_block_events",
            Response::BlockEvents {
                items: vec![BlockEvent {
                    at_unix_ms: 1_700_000_001_000,
                    kind: TargetKind::Site,
                    target: "youtube.com".to_owned(),
                }],
                next_offset: None,
            },
        ),
        error(
            "res_error_strict_session_locked",
            ProtocolError::StrictSessionLocked {
                reason: LockReason::UntilTimerEnds,
            },
        ),
        error(
            "res_error_validation",
            ProtocolError::Validation {
                field: "blocklist.name".to_owned(),
                kind: ValidationKind::Empty,
            },
        ),
        error("res_error_not_found", ProtocolError::NotFound),
        error(
            "res_error_unsupported_version",
            ProtocolError::UnsupportedVersion { supported: vec![1] },
        ),
        error("res_error_malformed", ProtocolError::Malformed),
        error("res_error_invalid_state", ProtocolError::InvalidState),
        error("res_error_internal", ProtocolError::Internal),
        evt("evt_state_changed", Event::StateChanged { state: state() }),
        evt(
            "evt_target_blocked",
            Event::TargetBlocked {
                event: BlockEvent {
                    at_unix_ms: 1_700_000_001_000,
                    kind: TargetKind::App,
                    target: "Steam".to_owned(),
                },
            },
        ),
        evt(
            "evt_blocklist_changed",
            Event::BlocklistChanged {
                id: "work".to_owned(),
            },
        ),
        evt(
            "evt_session_ended",
            Event::SessionEnded {
                outcome: Outcome::Stopped,
            },
        ),
    ]
}

/// The golden frames, by name.
fn golden() -> Vec<(String, Value)> {
    let document: Value = serde_json::from_str(GOLDEN).expect("the golden file is JSON");
    document
        .as_array()
        .expect("the golden file is an array")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().expect("a name").to_owned(),
                entry["frame"].clone(),
            )
        })
        .collect()
}

fn golden_frame(name: &str) -> Value {
    golden()
        .into_iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, frame)| frame)
        .unwrap_or_else(|| panic!("no golden frame named {name}"))
}

fn bytes(frame: &Value) -> Vec<u8> {
    serde_json::to_vec(frame).expect("a JSON value serializes")
}

fn without_newline(frame: &[u8]) -> &[u8] {
    frame.strip_suffix(b"\n").expect("ends with a newline")
}

/// Parses an encoded frame, checking that it is exactly one line.
fn parse_line(frame: &[u8]) -> Value {
    let body = without_newline(frame);
    assert!(!body.contains(&b'\n'), "no newline inside a frame");
    serde_json::from_slice(body).expect("a frame is JSON")
}

fn rejected(frame: &[u8]) -> DecodeError {
    decode(frame).expect_err("the frame must be rejected")
}

fn assert_malformed(frame: &[u8]) {
    assert_eq!(
        rejected(frame).error(),
        &ProtocolError::Malformed,
        "frame: {}",
        String::from_utf8_lossy(frame.get(..frame.len().min(200)).expect("in range"))
    );
}

fn validation_of(frame: &[u8]) -> (String, ValidationKind) {
    match rejected(frame).error() {
        ProtocolError::Validation { field, kind } => (field.clone(), *kind),
        other => panic!("expected a validation error, got {other:?}"),
    }
}

fn get_state_json() -> Value {
    json!({ "v": 1, "id": "c1", "kind": "request", "payload": { "type": "get_state" } })
}

fn padded_to(len: usize) -> Vec<u8> {
    let mut frame = bytes(&get_state_json());
    assert!(frame.len() <= len);
    frame.resize(len, b' ');
    frame
}

fn request_of(frame: &Value) -> Request {
    request_from(&bytes(frame))
}

fn no_frames() -> Vec<Result<Vec<u8>, ProtocolError>> {
    Vec::new()
}

fn request_from(frame: &[u8]) -> Request {
    match decode(frame).expect("decodes").message() {
        Message::Request(request) => request.clone(),
        other => panic!("expected a request, got {other:?}"),
    }
}

fn long_text(len: usize) -> String {
    "a".repeat(len)
}

/// A blocklist with every value at its maximum.
fn largest_blocklist() -> Blocklist {
    let path_len = MAX_APP_ID_LEN.checked_sub(3).expect("room for a drive");
    let pattern_len = MAX_PATTERN_LEN
        .checked_sub("example.com/".len())
        .expect("room for a host");
    let target = Target::App {
        name: "n".repeat(MAX_NAME_LEN),
        ids: AppIdentifiers {
            windows_exe: Some("e".repeat(MAX_APP_ID_LEN)),
            windows_path: Some(format!("C:\\{}", "p".repeat(path_len))),
            android_package: Some("a".repeat(MAX_APP_ID_LEN)),
            ios_token: Some("i".repeat(MAX_APP_ID_LEN)),
        },
    };
    Blocklist {
        id: "x".repeat(MAX_ID_LEN),
        name: "b".repeat(MAX_NAME_LEN),
        color: "#abcdef".to_owned(),
        targets: vec![target; MAX_TARGETS],
        exceptions: vec![
            SitePattern::new(format!("example.com/{}", "q".repeat(pattern_len)));
            MAX_EXCEPTIONS
        ],
        block_during_breaks: true,
    }
}

/// A golden frame, serialized after `edit` changed it.
fn edited(name: &str, edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut frame = golden_frame(name);
    edit(&mut frame);
    bytes(&frame)
}

// ---------------------------------------------------------------------------
// S: serialization
// ---------------------------------------------------------------------------

#[test]
fn every_message_encodes_to_its_golden_frame() {
    let golden = golden();
    for (name, envelope) in samples() {
        let wire = parse_line(&encode(&envelope).expect("encodes"));
        let (_, expected) = golden
            .iter()
            .find(|(candidate, _)| candidate == name)
            .unwrap_or_else(|| panic!("no golden frame named {name}"));
        assert_eq!(&wire, expected, "wire format of {name}");
    }
}

#[test]
fn every_golden_frame_decodes_to_its_message() {
    let golden = golden();
    for (name, envelope) in samples() {
        let (_, frame) = golden
            .iter()
            .find(|(candidate, _)| candidate == name)
            .unwrap_or_else(|| panic!("no golden frame named {name}"));
        let decoded = decode(&bytes(frame)).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(decoded, envelope, "decoding {name}");
    }
}

#[test]
fn the_fixture_has_exactly_the_sample_messages() {
    let in_fixture: Vec<String> = golden().into_iter().map(|(name, _)| name).collect();
    let in_samples: Vec<String> = samples()
        .into_iter()
        .map(|(name, _)| name.to_owned())
        .collect();
    let unique: BTreeSet<&String> = in_fixture.iter().collect();
    assert_eq!(unique.len(), in_fixture.len(), "names are unique");
    assert_eq!(in_fixture, in_samples);

    // Every request of docs/architecture.md (plus resume and the block-event
    // and blocklist reads) is in the contract.
    let types: BTreeSet<String> = golden()
        .iter()
        .filter(|(_, frame)| frame["kind"] == "request")
        .map(|(_, frame)| {
            frame["payload"]["type"]
                .as_str()
                .expect("a type")
                .to_owned()
        })
        .collect();
    for expected in [
        "get_state",
        "start_session",
        "stop_session",
        "pause_session",
        "resume_session",
        "skip_break",
        "list_blocklists",
        "get_blocklist",
        "upsert_blocklist",
        "delete_blocklist",
        "get_settings",
        "update_settings",
        "list_installed_apps",
        "get_history",
        "get_block_events",
        "subscribe",
    ] {
        assert!(types.contains(expected), "no golden request {expected}");
    }
}

#[test]
fn frame_too_large_error_has_its_wire_shape() {
    // Its number is the limit, which the golden file can't hold.
    let max = u32::try_from(MAX_FRAME_LEN).expect("the limit fits in a u32");
    let envelope = Envelope::new(
        mid("t-limit"),
        Message::Response(Response::Error {
            error: ProtocolError::FrameTooLarge { max },
        }),
    );
    assert_eq!(
        parse_line(&encode(&envelope).expect("encodes")),
        json!({
            "v": 1, "id": "t-limit", "kind": "response",
            "payload": { "type": "error", "error": { "code": "frame_too_large", "max": max } }
        })
    );
    assert_eq!(
        decode(&bytes(&parse_line(&encode(&envelope).expect("encodes")))).expect("decodes"),
        envelope
    );
}

#[test]
fn every_encoded_frame_is_one_line_with_the_envelope() {
    for (name, envelope) in samples() {
        let wire = parse_line(&encode(&envelope).expect("encodes"));
        assert_eq!(wire["v"], json!(PROTOCOL_VERSION), "{name}");
        assert_eq!(wire["id"], json!(envelope.id().as_str()), "{name}");
        let kind = match envelope.message() {
            Message::Request(_) => "request",
            Message::Response(_) => "response",
            Message::Event(_) => "event",
        };
        assert_eq!(wire["kind"], json!(kind), "{name}");
        assert!(wire["payload"]["type"].is_string(), "{name}");
    }
    assert_eq!(PROTOCOL_VERSION, 1);
}

#[test]
fn events_carry_the_protocol_version() {
    let mut events = 0_u32;
    for (name, envelope) in samples() {
        if matches!(envelope.message(), Message::Event(_)) {
            events += 1;
            assert_eq!(
                parse_line(&encode(&envelope).expect("encodes"))["v"],
                1,
                "{name}"
            );
        }
    }
    assert_eq!(events, 4);
}

#[test]
fn enum_wire_names_are_stable() {
    fn state_with(phase: Phase) -> Value {
        let mut session = state();
        session.phase = phase;
        let envelope = Envelope::new(
            mid("e"),
            Message::Event(Event::StateChanged { state: session }),
        );
        parse_line(&encode(&envelope).expect("encodes"))["payload"]["state"]["phase"].clone()
    }
    assert_eq!(state_with(Phase::Idle), "idle");
    assert_eq!(state_with(Phase::Focus), "focus");
    assert_eq!(state_with(Phase::ShortBreak), "short_break");
    assert_eq!(state_with(Phase::LongBreak), "long_break");

    let ended = |outcome| {
        let envelope = Envelope::new(mid("e"), Message::Event(Event::SessionEnded { outcome }));
        parse_line(&encode(&envelope).expect("encodes"))["payload"]["outcome"].clone()
    };
    assert_eq!(ended(Outcome::Completed), "completed");
    assert_eq!(ended(Outcome::Stopped), "stopped");

    let blocked = |kind| {
        let envelope = Envelope::new(
            mid("e"),
            Message::Event(Event::TargetBlocked {
                event: BlockEvent {
                    at_unix_ms: 1,
                    kind,
                    target: "t".to_owned(),
                },
            }),
        );
        parse_line(&encode(&envelope).expect("encodes"))["payload"]["event"]["kind"].clone()
    };
    assert_eq!(blocked(TargetKind::Site), "site");
    assert_eq!(blocked(TargetKind::App), "app");

    let ranged = |range| {
        let envelope = Envelope::new(
            mid("e"),
            Message::Request(Request::GetHistory {
                range,
                page: page(0, 10),
            }),
        );
        parse_line(&encode(&envelope).expect("encodes"))["payload"]["range"].clone()
    };
    assert_eq!(ranged(HistoryRange::Today), "today");
    assert_eq!(ranged(HistoryRange::Week), "week");
    assert_eq!(ranged(HistoryRange::Month), "month");
    assert_eq!(ranged(HistoryRange::All), "all");

    let kinds = |kind| {
        let envelope = Envelope::new(
            mid("e"),
            Message::Response(Response::Error {
                error: ProtocolError::Validation {
                    field: "f".to_owned(),
                    kind,
                },
            }),
        );
        parse_line(&encode(&envelope).expect("encodes"))["payload"]["error"]["kind"].clone()
    };
    for (kind, name) in [
        (ValidationKind::Empty, "empty"),
        (ValidationKind::TooLong, "too_long"),
        (ValidationKind::TooMany, "too_many"),
        (ValidationKind::ControlCharacter, "control_character"),
        (ValidationKind::NotAnId, "not_an_id"),
        (ValidationKind::NotAColor, "not_a_color"),
        (ValidationKind::NotAFileName, "not_a_file_name"),
        (ValidationKind::NotADrivePath, "not_a_drive_path"),
        (ValidationKind::NoIdentifier, "no_identifier"),
        (ValidationKind::NotASitePattern, "not_a_site_pattern"),
        (ValidationKind::OutOfRange, "out_of_range"),
        (ValidationKind::Invalid, "invalid"),
    ] {
        assert_eq!(kinds(kind), name);
    }
}

#[test]
fn error_codes_are_stable() {
    let max = u32::try_from(MAX_FRAME_LEN).expect("fits");
    for (error, code) in [
        (
            ProtocolError::StrictSessionLocked {
                reason: LockReason::UntilTimerEnds,
            },
            "strict_session_locked",
        ),
        (
            ProtocolError::Validation {
                field: "f".to_owned(),
                kind: ValidationKind::Empty,
            },
            "validation",
        ),
        (ProtocolError::NotFound, "not_found"),
        (
            ProtocolError::UnsupportedVersion { supported: vec![1] },
            "unsupported_version",
        ),
        (ProtocolError::FrameTooLarge { max }, "frame_too_large"),
        (ProtocolError::Malformed, "malformed"),
        (ProtocolError::InvalidState, "invalid_state"),
        (ProtocolError::Internal, "internal"),
    ] {
        assert_eq!(error.code(), code);
    }
    fn is_error<E: std::error::Error + Send + Sync + 'static>() {}
    is_error::<ProtocolError>();
    is_error::<DecodeError>();
}

#[test]
fn phase_names_decode_to_phases() {
    for (name, phase) in [
        ("idle", Phase::Idle),
        ("focus", Phase::Focus),
        ("short_break", Phase::ShortBreak),
        ("long_break", Phase::LongBreak),
    ] {
        let mut frame = golden_frame("evt_state_changed");
        frame["payload"]["state"]["phase"] = json!(name);
        let decoded = decode(&bytes(&frame)).expect("decodes");
        match decoded.message() {
            Message::Event(Event::StateChanged { state }) => assert_eq!(state.phase, phase),
            other => panic!("unexpected message {other:?}"),
        }
    }
    assert_eq!(Phase::from(CorePhase::Idle), Phase::Idle);
    assert_eq!(Phase::from(CorePhase::Focus), Phase::Focus);
    assert_eq!(Phase::from(CorePhase::ShortBreak), Phase::ShortBreak);
    assert_eq!(Phase::from(CorePhase::LongBreak), Phase::LongBreak);
    assert_eq!(
        LockReason::from(CoreLockReason::UntilTimerEnds),
        LockReason::UntilTimerEnds
    );
}

#[test]
fn whatever_decodes_re_encodes_to_the_same_message() {
    for (name, envelope) in samples() {
        let frame = encode(&envelope).expect("encodes");
        let again =
            decode(without_newline(&frame)).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(again, envelope, "{name}");
        assert_eq!(encode(&again).expect("encodes"), frame, "{name}");
    }
}

// ---------------------------------------------------------------------------
// V: version compatibility
// ---------------------------------------------------------------------------

#[test]
fn a_newer_version_is_rejected_with_unsupported_version() {
    for version in [json!(0), json!(2), json!(3), json!(99), json!(u64::MAX)] {
        let mut frame = get_state_json();
        frame["v"] = version.clone();
        let error = rejected(&bytes(&frame));
        assert_eq!(
            error.error(),
            &ProtocolError::UnsupportedVersion { supported: vec![1] },
            "version {version}"
        );
        assert_eq!(
            error.id(),
            Some(&mid("c1")),
            "the id is kept: version {version}"
        );
        // The answer is itself a frame of the current version.
        let response = parse_line(&encode(&error.into_response()).expect("encodes"));
        assert_eq!(response["v"], 1);
        assert_eq!(response["id"], "c1");
        assert_eq!(response["kind"], "response");
        assert_eq!(
            response["payload"],
            json!({ "type": "error", "error": { "code": "unsupported_version", "supported": [1] } })
        );
    }
}

#[test]
fn the_version_is_checked_before_the_payload() {
    for frame in [
        json!({ "v": 2, "id": "c1", "kind": "request", "payload": { "type": "brand_new" } }),
        json!({ "v": 2, "id": "c1", "kind": "teleport", "payload": 5 }),
        json!({ "v": 2, "id": "c1" }),
        json!({ "v": 2 }),
    ] {
        let error = rejected(&bytes(&frame));
        assert_eq!(
            error.error(),
            &ProtocolError::UnsupportedVersion { supported: vec![1] },
            "{frame}"
        );
    }
    let error = rejected(&bytes(&json!({ "v": 2 })));
    assert_eq!(error.id(), None);
    let error = rejected(&bytes(
        &json!({ "v": 2, "id": "has space", "kind": "request" }),
    ));
    assert_eq!(error.id(), None, "an invalid id is never kept");
}

#[test]
fn a_missing_or_non_integer_version_is_malformed() {
    for version in [
        json!("1"),
        json!(1.5),
        json!(null),
        json!(true),
        json!(-1),
        json!([1]),
        json!({ "major": 1 }),
    ] {
        let mut frame = get_state_json();
        frame["v"] = version;
        assert_malformed(&bytes(&frame));
    }
    assert_malformed(br#"{"id":"c1","kind":"request","payload":{"type":"get_state"}}"#);
    // Not an object at all.
    for frame in [&b"[]"[..], b"1", b"\"v\"", b"null", b"true"] {
        assert_malformed(frame);
    }
}

#[test]
fn unknown_fields_are_ignored_at_every_level() {
    let baseline = decode(&bytes(&golden_frame("req_start_session"))).expect("decodes");
    let mut frame = golden_frame("req_start_session");
    frame["future"] = json!({ "nested": [1, 2, { "deep": true }] });
    frame["payload"]["priority"] = json!("high");
    frame["payload"]["preset"]["tomato"] = json!(1);
    frame["payload"]["lock"]["strength"] = json!(11);
    assert_eq!(decode(&bytes(&frame)).expect("decodes"), baseline);

    let baseline = decode(&bytes(&golden_frame("req_upsert_blocklist"))).expect("decodes");
    let mut frame = golden_frame("req_upsert_blocklist");
    frame["payload"]["blocklist"]["icon"] = json!("star");
    frame["payload"]["blocklist"]["targets"][0]["weight"] = json!(3);
    frame["payload"]["blocklist"]["targets"][1]["ids"]["linux_binary"] = json!("steam");
    assert_eq!(decode(&bytes(&frame)).expect("decodes"), baseline);

    // Responses and events are read the same way, so a newer service's
    // additions don't break an older client.
    let baseline = decode(&bytes(&golden_frame("evt_state_changed"))).expect("decodes");
    let mut frame = golden_frame("evt_state_changed");
    frame["payload"]["state"]["eta_ms"] = json!(5);
    frame["payload"]["extra"] = json!("x");
    assert_eq!(decode(&bytes(&frame)).expect("decodes"), baseline);
    let baseline = decode(&bytes(&golden_frame("res_history"))).expect("decodes");
    let mut frame = golden_frame("res_history");
    frame["payload"]["items"][0]["streak"] = json!(3);
    assert_eq!(decode(&bytes(&frame)).expect("decodes"), baseline);
}

#[test]
fn optional_fields_may_be_absent_or_null() {
    let without = decode(&edited("req_stop_session_without_text", |frame| {
        frame["payload"]
            .as_object_mut()
            .expect("object")
            .remove("text");
    }))
    .expect("decodes");
    assert_eq!(
        without,
        decode(&bytes(&golden_frame("req_stop_session_without_text"))).expect("decodes")
    );

    let frame = edited("res_blocklists", |frame| {
        frame["payload"]
            .as_object_mut()
            .expect("object")
            .remove("next_offset");
    });
    match decode(&frame).expect("decodes").message() {
        Message::Response(Response::Blocklists { next_offset, .. }) => {
            assert_eq!(*next_offset, None);
        }
        other => panic!("unexpected message {other:?}"),
    }

    let frame = edited("evt_state_changed", |frame| {
        let state = frame["payload"]["state"].as_object_mut().expect("object");
        state.remove("lock");
        state.remove("blocklist_id");
    });
    match decode(&frame).expect("decodes").message() {
        Message::Event(Event::StateChanged { state }) => {
            assert_eq!(state.lock, None);
            assert_eq!(state.blocklist_id, None);
        }
        other => panic!("unexpected message {other:?}"),
    }

    // Required fields stay required: a missing lock must never turn a strict
    // session into an unlocked one.
    let frame = edited("req_start_session", |frame| {
        frame["payload"]
            .as_object_mut()
            .expect("object")
            .remove("lock");
    });
    assert_malformed(&frame);
    let frame = edited("req_start_session", |frame| {
        frame["payload"]["lock"] = json!(null);
    });
    assert_malformed(&frame);
}

#[test]
fn an_unknown_message_type_is_malformed_and_keeps_the_id() {
    for kind in ["request", "response", "event"] {
        let frame = json!({
            "v": 1, "id": "c2", "kind": kind,
            "payload": { "type": "brand_new_thing", "x": 1 }
        });
        let error = rejected(&bytes(&frame));
        assert_eq!(error.error(), &ProtocolError::Malformed, "{kind}");
        assert_eq!(error.id(), Some(&mid("c2")), "{kind}");
    }
    // The reply to it is a frame the peer can read.
    let error = rejected(&bytes(&json!({
        "v": 1, "id": "c2", "kind": "request", "payload": { "type": "brand_new_thing" }
    })));
    assert_eq!(
        parse_line(&encode(&error.into_response()).expect("encodes")),
        json!({
            "v": 1, "id": "c2", "kind": "response",
            "payload": { "type": "error", "error": { "code": "malformed" } }
        })
    );
    // A payload with no type, or a type that isn't text.
    for payload in [
        json!({}),
        json!({ "type": 5 }),
        json!({ "type": null }),
        json!(5),
        json!(null),
    ] {
        let frame = json!({ "v": 1, "id": "c3", "kind": "request", "payload": payload });
        assert_malformed(&bytes(&frame));
    }
}

#[test]
fn an_unknown_event_does_not_break_the_stream() {
    let unknown = bytes(&json!({
        "v": 1, "id": "ev1", "kind": "event", "payload": { "type": "from_the_future" }
    }));
    let known = bytes(&golden_frame("evt_blocklist_changed"));
    let mut stream = unknown.clone();
    stream.push(b'\n');
    stream.extend_from_slice(&known);
    stream.push(b'\n');
    let mut reader = FrameReader::new();
    let frames = reader.push(&stream);
    assert_eq!(frames, vec![Ok(unknown.clone()), Ok(known.clone())]);
    assert_malformed(&unknown);
    decode(&known).expect("the next frame still decodes");
}

#[test]
fn a_kind_that_does_not_match_the_payload_is_malformed() {
    let mut frame = golden_frame("req_get_state");
    frame["kind"] = json!("response");
    assert_malformed(&bytes(&frame));
    let mut frame = golden_frame("res_ok");
    frame["kind"] = json!("request");
    assert_malformed(&bytes(&frame));
    let mut frame = golden_frame("evt_session_ended");
    frame["kind"] = json!("response");
    assert_malformed(&bytes(&frame));
    for kind in [json!("notification"), json!(""), json!(null), json!(5)] {
        let mut frame = golden_frame("req_get_state");
        frame["kind"] = kind;
        assert_malformed(&bytes(&frame));
    }
    let mut frame = golden_frame("req_get_state");
    frame.as_object_mut().expect("object").remove("kind");
    assert_malformed(&bytes(&frame));
}

// ---------------------------------------------------------------------------
// D: validation at decode
// ---------------------------------------------------------------------------

#[test]
fn an_invalid_blocklist_is_rejected_at_decode() {
    let upsert = |edit: &dyn Fn(&mut Value)| {
        edited("req_upsert_blocklist", |frame| {
            edit(&mut frame["payload"]["blocklist"])
        })
    };
    let many_targets: Vec<Value> = (0..=MAX_TARGETS)
        .map(|_| json!({ "kind": "site", "pattern": "example.com" }))
        .collect();
    let cases: Vec<(Vec<u8>, &str, ValidationKind)> = vec![
        (
            upsert(&|b| b["name"] = json!("")),
            "blocklist.name",
            ValidationKind::Empty,
        ),
        (
            upsert(&|b| b["name"] = json!(long_text(MAX_NAME_LEN + 1))),
            "blocklist.name",
            ValidationKind::TooLong,
        ),
        (
            upsert(&|b| b["name"] = json!("two\nlines")),
            "blocklist.name",
            ValidationKind::ControlCharacter,
        ),
        (
            upsert(&|b| b["color"] = json!("red")),
            "blocklist.color",
            ValidationKind::NotAColor,
        ),
        (
            upsert(&|b| b["id"] = json!("bad id")),
            "blocklist.id",
            ValidationKind::NotAnId,
        ),
        (
            upsert(&|b| b["id"] = json!("")),
            "blocklist.id",
            ValidationKind::Empty,
        ),
        (
            upsert(&|b| b["id"] = json!(long_text(MAX_ID_LEN + 1))),
            "blocklist.id",
            ValidationKind::TooLong,
        ),
        (
            upsert(&|b| b["targets"][0]["pattern"] = json!("https://youtube.com")),
            "blocklist.targets[0].pattern",
            ValidationKind::NotASitePattern,
        ),
        (
            upsert(&|b| b["exceptions"][0] = json!("")),
            "blocklist.exceptions[0]",
            ValidationKind::Empty,
        ),
        (
            upsert(&|b| b["targets"][1]["ids"] = json!({})),
            "blocklist.targets[1].ids",
            ValidationKind::NoIdentifier,
        ),
        (
            upsert(&|b| b["targets"][1]["ids"]["windows_path"] = json!("\\\\server\\share\\x.exe")),
            "blocklist.targets[1].ids.windows_path",
            ValidationKind::NotADrivePath,
        ),
        (
            upsert(&|b| b["targets"][1]["ids"]["windows_exe"] = json!("dir/steam.exe")),
            "blocklist.targets[1].ids.windows_exe",
            ValidationKind::NotAFileName,
        ),
        (
            upsert(&|b| b["targets"] = json!(many_targets)),
            "blocklist.targets",
            ValidationKind::TooMany,
        ),
    ];
    for (frame, field, kind) in cases {
        assert_eq!(validation_of(&frame), (field.to_owned(), kind), "{field}");
    }
    // A blocklist in the wrong shape is malformed, not a validation error.
    assert_malformed(&edited("req_upsert_blocklist", |frame| {
        frame["payload"]["blocklist"]["targets"] = json!("youtube.com");
    }));
    assert_malformed(&edited("req_upsert_blocklist", |frame| {
        frame["payload"]
            .as_object_mut()
            .expect("object")
            .remove("blocklist");
    }));
}

#[test]
fn validated_blocklist_only_holds_valid_blocklists() {
    let mut invalid = blocklist();
    invalid.color = "red".to_owned();
    let error = ValidatedBlocklist::new(invalid).expect_err("an invalid blocklist");
    assert_eq!(error.field(), "color");
    assert_eq!(error.kind(), ValidationErrorKind::NotAColor);

    let valid = validated();
    assert_eq!(valid.as_blocklist(), &blocklist());
    assert_eq!(valid.clone().into_blocklist(), blocklist());
}

#[test]
fn a_blocklist_at_every_model_maximum_is_accepted() {
    let valid =
        ValidatedBlocklist::new(largest_blocklist()).expect("the largest blocklist is valid");
    let envelope = Envelope::new(
        mid("big"),
        Message::Request(Request::UpsertBlocklist { blocklist: valid }),
    );
    let frame = encode(&envelope).expect("the largest blocklist fits in a frame");
    assert!(
        frame.len() > 2_000_000,
        "this is the large case: {} bytes",
        frame.len()
    );
    assert_eq!(decode(without_newline(&frame)).expect("decodes"), envelope);
}

#[test]
fn a_random_text_lock_out_of_range_is_rejected() {
    for len in [0_u32, MAX_RANDOM_TEXT_LEN + 1, 1_000, u32::MAX] {
        let frame = edited("req_start_session", |frame| {
            frame["payload"]["lock"] = json!({ "kind": "random_text", "len": len });
        });
        assert_eq!(
            validation_of(&frame),
            ("lock".to_owned(), ValidationKind::OutOfRange),
            "len {len}"
        );
    }
    for len in [1_u32, 8, MAX_RANDOM_TEXT_LEN] {
        let frame = edited("req_start_session", |frame| {
            frame["payload"]["lock"] = json!({ "kind": "random_text", "len": len });
        });
        match request_from(&frame) {
            Request::StartSession { lock, .. } => assert_eq!(lock, LockKind::RandomText { len }),
            other => panic!("unexpected request {other:?}"),
        }
    }
    for (lock, expected) in [
        (json!({ "kind": "none" }), LockKind::None),
        (
            json!({ "kind": "until_timer_ends" }),
            LockKind::UntilTimerEnds,
        ),
    ] {
        let mut frame = golden_frame("req_start_session");
        frame["payload"]["lock"] = lock;
        match request_of(&frame) {
            Request::StartSession { lock, .. } => assert_eq!(lock, expected),
            other => panic!("unexpected request {other:?}"),
        }
    }
}

#[test]
fn a_lock_with_inconsistent_fields_is_malformed() {
    for lock in [
        json!({ "kind": "random_text" }),
        json!({ "kind": "none", "len": 3 }),
        json!({ "kind": "until_timer_ends", "len": 3 }),
        json!({ "kind": "sleep" }),
        json!({}),
        json!("none"),
    ] {
        let frame = edited("req_start_session", |frame| frame["payload"]["lock"] = lock);
        assert_malformed(&frame);
    }
}

#[test]
fn presets_out_of_range_are_rejected() {
    let maxima = [
        ("focus_min", MAX_FOCUS_MIN),
        ("short_break_min", MAX_SHORT_BREAK_MIN),
        ("long_break_min", MAX_LONG_BREAK_MIN),
        ("cycles", MAX_CYCLES),
    ];
    for (field, max) in maxima {
        for bad in [0, max + 1, u32::MAX] {
            let frame = edited("req_start_session", |frame| {
                frame["payload"]["preset"][field] = json!(bad);
            });
            assert_eq!(
                validation_of(&frame),
                (format!("preset.{field}"), ValidationKind::OutOfRange),
                "{field} = {bad}"
            );
            let frame = edited("req_update_settings", |frame| {
                frame["payload"]["settings"]["default_preset"][field] = json!(bad);
            });
            assert_eq!(
                validation_of(&frame),
                (
                    format!("settings.default_preset.{field}"),
                    ValidationKind::OutOfRange
                ),
                "settings {field} = {bad}"
            );
        }
        for good in [1, max] {
            let mut frame = golden_frame("req_start_session");
            frame["payload"]["preset"][field] = json!(good);
            request_of(&frame);
        }
    }
    // Wrong types are malformed.
    assert_malformed(&edited("req_start_session", |frame| {
        frame["payload"]["preset"]["focus_min"] = json!("25");
    }));
    assert_malformed(&edited("req_start_session", |frame| {
        frame["payload"]["preset"] = json!({ "focus_min": 25 });
    }));
}

#[test]
fn blocklist_ids_are_validated() {
    let bad: Vec<(String, ValidationKind)> = vec![
        (String::new(), ValidationKind::Empty),
        ("has space".to_owned(), ValidationKind::NotAnId),
        ("a/b".to_owned(), ValidationKind::NotAnId),
        ("caf\u{e9}".to_owned(), ValidationKind::NotAnId),
        ("under_score".to_owned(), ValidationKind::NotAnId),
        (long_text(MAX_ID_LEN + 1), ValidationKind::TooLong),
    ];
    for (id, kind) in bad {
        let frame = edited("req_start_session", |frame| {
            frame["payload"]["blocklist_id"] = json!(id)
        });
        assert_eq!(
            validation_of(&frame),
            ("blocklist_id".to_owned(), kind),
            "{id:?}"
        );
        for name in ["req_get_blocklist", "req_delete_blocklist"] {
            let frame = edited(name, |frame| frame["payload"]["id"] = json!(id));
            assert_eq!(
                validation_of(&frame),
                ("id".to_owned(), kind),
                "{name} {id:?}"
            );
        }
    }
    for good in [
        "a",
        "0",
        "123e4567-e89b-12d3-a456-426614174000",
        &long_text(MAX_ID_LEN),
    ] {
        let frame = edited("req_get_blocklist", |frame| {
            frame["payload"]["id"] = json!(good)
        });
        decode(&frame).expect("a valid id");
    }
}

#[test]
fn page_limits_are_capped() {
    assert_eq!(MAX_PAGE_LIMIT, 100);
    for name in [
        "req_list_blocklists",
        "req_list_installed_apps",
        "req_get_history",
        "req_get_block_events",
    ] {
        for bad in [0, MAX_PAGE_LIMIT + 1, 1_000_000, u32::MAX] {
            let frame = edited(name, |frame| frame["payload"]["page"]["limit"] = json!(bad));
            assert_eq!(
                validation_of(&frame),
                ("page.limit".to_owned(), ValidationKind::OutOfRange),
                "{name} limit {bad}"
            );
        }
        for good in [1, MAX_PAGE_LIMIT] {
            let frame = edited(name, |frame| {
                frame["payload"]["page"]["limit"] = json!(good);
                frame["payload"]["page"]["offset"] = json!(u32::MAX);
            });
            decode(&frame).expect("a page within the cap");
        }
        // Without a page there is no unbounded list.
        let frame = edited(name, |frame| {
            frame["payload"]
                .as_object_mut()
                .expect("object")
                .remove("page");
        });
        assert_malformed(&frame);
        // A negative or fractional limit is not a number of items.
        for bad in [json!(-1), json!(2.5), json!("10")] {
            let frame = edited(name, |frame| frame["payload"]["page"]["limit"] = bad);
            assert_malformed(&frame);
        }
    }
    assert!(PageRequest::new(0, 0).is_err());
    assert!(PageRequest::new(0, MAX_PAGE_LIMIT + 1).is_err());
    let ok = PageRequest::new(7, MAX_PAGE_LIMIT).expect("within the cap");
    assert_eq!((ok.offset(), ok.limit()), (7, MAX_PAGE_LIMIT));
    for range in ["today", "week", "month", "all"] {
        let frame = edited("req_get_history", |frame| {
            frame["payload"]["range"] = json!(range)
        });
        decode(&frame).expect("a named range");
    }
    for range in [
        json!("year"),
        json!(null),
        json!(7),
        json!({ "from": 0, "to": 1 }),
    ] {
        assert_malformed(&edited("req_get_history", |frame| {
            frame["payload"]["range"] = range
        }));
    }
}

#[test]
fn stop_text_is_capped() {
    assert_eq!(MAX_STOP_TEXT_BYTES, 1024);
    let frame = edited("req_stop_session_with_text", |frame| {
        frame["payload"]["text"] = json!(long_text(MAX_STOP_TEXT_BYTES));
    });
    decode(&frame).expect("text at the cap");
    for text in [
        long_text(MAX_STOP_TEXT_BYTES + 1),
        // Counted in bytes, not characters.
        "\u{e9}".repeat(MAX_STOP_TEXT_BYTES / 2 + 1),
    ] {
        let frame = edited("req_stop_session_with_text", |frame| {
            frame["payload"]["text"] = json!(text)
        });
        assert_eq!(
            validation_of(&frame),
            ("text".to_owned(), ValidationKind::TooLong)
        );
    }
    // An empty text is a wrong text for the service to judge, not malformed.
    let frame = edited("req_stop_session_with_text", |frame| {
        frame["payload"]["text"] = json!("")
    });
    assert_eq!(
        request_from(&frame),
        Request::StopSession {
            text: Some(String::new())
        }
    );
    assert_malformed(&edited("req_stop_session_with_text", |frame| {
        frame["payload"]["text"] = json!(5);
    }));
}

#[test]
fn message_ids_are_validated() {
    for id in ["", "has space", "caf\u{e9}", "a/b", "line\nbreak"] {
        assert!(MessageId::new(id).is_err(), "{id:?}");
    }
    assert!(MessageId::new(&long_text(65)).is_err());
    assert_eq!(
        MessageId::new(&long_text(64)).expect("64 bytes").as_str(),
        long_text(64)
    );
    assert_eq!(
        MessageId::new("550e8400-e29b-41d4-a716-446655440000")
            .expect("a uuid")
            .as_str(),
        "550e8400-e29b-41d4-a716-446655440000"
    );

    for id in [
        json!(""),
        json!("has space"),
        json!(long_text(65)),
        json!(123),
        json!(null),
        json!(["a"]),
    ] {
        let mut frame = get_state_json();
        frame["id"] = id;
        let error = rejected(&bytes(&frame));
        assert_eq!(error.error(), &ProtocolError::Malformed);
        assert_eq!(error.id(), None, "an invalid id is never echoed");
    }
    let mut frame = get_state_json();
    frame.as_object_mut().expect("object").remove("id");
    let error = rejected(&bytes(&frame));
    assert_eq!(error.error(), &ProtocolError::Malformed);
    // With no usable id the answer goes out under the reserved one.
    assert_eq!(
        parse_line(&encode(&error.into_response()).expect("encodes")),
        json!({
            "v": 1, "id": "unknown", "kind": "response",
            "payload": { "type": "error", "error": { "code": "malformed" } }
        })
    );
}

#[test]
fn validation_kinds_map_from_the_core() {
    for (core, expected) in [
        (ValidationErrorKind::Empty, ValidationKind::Empty),
        (
            ValidationErrorKind::TooLong { len: 2, max: 1 },
            ValidationKind::TooLong,
        ),
        (
            ValidationErrorKind::TooMany { count: 2, max: 1 },
            ValidationKind::TooMany,
        ),
        (
            ValidationErrorKind::ControlCharacter,
            ValidationKind::ControlCharacter,
        ),
        (ValidationErrorKind::NotAnId, ValidationKind::NotAnId),
        (ValidationErrorKind::NotAColor, ValidationKind::NotAColor),
        (
            ValidationErrorKind::NotAFileName,
            ValidationKind::NotAFileName,
        ),
        (
            ValidationErrorKind::NotADrivePath,
            ValidationKind::NotADrivePath,
        ),
        (
            ValidationErrorKind::NoIdentifier,
            ValidationKind::NoIdentifier,
        ),
        (
            ValidationErrorKind::NotASitePattern,
            ValidationKind::NotASitePattern,
        ),
    ] {
        assert_eq!(ValidationKind::from(core), expected);
    }
}

// ---------------------------------------------------------------------------
// X: security by construction
// ---------------------------------------------------------------------------

#[test]
fn there_is_no_request_that_bypasses_the_session_rules() {
    // Nothing that ends, shortens, extends or re-times a session, moves a
    // clock, switches the lock off or checks a URL exists in this version.
    // The service judges the listed requests; nothing else can be asked.
    for name in [
        "force_stop",
        "stop",
        "pause",
        "cancel",
        "cancel_session",
        "end_session",
        "abort_session",
        "shorten_session",
        "extend_session",
        "skip_focus",
        "skip_phase",
        "set_remaining",
        "set_time",
        "set_clock",
        "advance_clock",
        "get_clock",
        "now",
        "set_preset",
        "set_lock",
        "unlock",
        "disable_strict",
        "clear_lock",
        "reset",
        "shutdown",
        "stop_service",
        "check_url",
        "match_url",
        "set_blocklist",
        "edit_active_blocklist",
        "restore_snapshot",
        "set_state",
    ] {
        let frame = json!({
            "v": 1, "id": "x1", "kind": "request",
            "payload": { "type": name, "text": "abcdef", "remaining_ms": 0 }
        });
        let error = rejected(&bytes(&frame));
        assert_eq!(
            error.error(),
            &ProtocolError::Malformed,
            "request type {name}"
        );
    }
    // A stop for a RandomText session carries the typed text; one for a
    // strict session carries nothing the service must trust.
    assert_eq!(
        request_of(&golden_frame("req_stop_session_with_text")),
        Request::StopSession {
            text: Some("k3Zq9x".to_owned())
        }
    );
}

#[test]
fn time_fields_in_a_request_are_ignored() {
    let baseline = decode(&bytes(&golden_frame("req_start_session"))).expect("decodes");
    let mut frame = golden_frame("req_start_session");
    for name in [
        "started_at",
        "now",
        "elapsed",
        "elapsed_ms",
        "remaining",
        "remaining_ms",
        "duration",
        "duration_ms",
        "clock",
        "wall",
        "monotonic",
        "deadline",
        "until",
        "ends_at",
        "at",
        "timestamp",
        "active",
        "offset",
    ] {
        frame["payload"][name] = json!(1);
        frame[name] = json!(1);
    }
    assert_eq!(decode(&bytes(&frame)).expect("decodes"), baseline);
    for name in [
        "req_stop_session_with_text",
        "req_pause_session",
        "req_skip_break",
        "req_get_state",
    ] {
        let baseline = decode(&bytes(&golden_frame(name))).expect("decodes");
        let mut frame = golden_frame(name);
        frame["payload"]["remaining_ms"] = json!(0);
        frame["payload"]["now"] = json!(0);
        frame["payload"]["clock"] = json!({ "wall": 1 });
        assert_eq!(decode(&bytes(&frame)).expect("decodes"), baseline, "{name}");
    }
}

#[test]
fn errors_never_echo_client_text() {
    const MARKER: &str = "SECRET-MARKER-9f2";
    let frames: Vec<Vec<u8>> = vec![
        bytes(&json!({ "v": 1, "id": "c1", "kind": "request", "payload": { "type": MARKER } })),
        bytes(&json!({ "v": 1, "id": "c1", "kind": MARKER, "payload": { "type": "get_state" } })),
        bytes(
            &json!({ "v": 1, "id": format!("{MARKER} !"), "kind": "request", "payload": { "type": "get_state" } }),
        ),
        bytes(
            &json!({ "v": MARKER, "id": "c1", "kind": "request", "payload": { "type": "get_state" } }),
        ),
        bytes(&json!({ "v": 2, "id": format!("{MARKER} !"), "kind": MARKER })),
        edited("req_start_session", |f| {
            f["payload"]["blocklist_id"] = json!(format!("{MARKER} !"))
        }),
        edited("req_upsert_blocklist", |f| {
            f["payload"]["blocklist"]["name"] = json!(format!("{MARKER}\n"))
        }),
        edited("req_upsert_blocklist", |f| {
            f["payload"]["blocklist"]["targets"][0]["pattern"] =
                json!(format!("https://{MARKER}.com"))
        }),
        edited("req_upsert_blocklist", |f| {
            f["payload"]["blocklist"]["color"] = json!(MARKER)
        }),
        edited("req_start_session", |f| {
            f["payload"]["preset"]["focus_min"] = json!(MARKER)
        }),
        edited("req_get_history", |f| f["payload"]["range"] = json!(MARKER)),
        edited("req_stop_session_with_text", |f| {
            f["payload"]["text"] = json!(MARKER.repeat(200))
        }),
        format!("{{\"v\":1,\"id\":\"c1\",\"{MARKER}\":").into_bytes(),
        format!("not json {MARKER}").into_bytes(),
    ];
    let leaks = [
        MARKER,
        "serde",
        "column",
        "panicked",
        ".rs",
        "unknown variant",
        "missing field",
    ];
    for frame in frames {
        let error = decode(&frame).expect_err("rejected");
        let shown = format!("{error} | {error:?} | {:?}", error.error());
        let response =
            String::from_utf8(encode(&error.into_response()).expect("encodes")).expect("UTF-8");
        for leak in leaks {
            assert!(!shown.contains(leak), "{leak:?} in {shown}");
            assert!(!response.contains(leak), "{leak:?} in {response}");
        }
        // The answer is bounded however large the request was.
        assert!(response.len() < 512, "{} bytes", response.len());
    }
}

#[test]
fn error_responses_carry_nothing_else() {
    for (name, envelope) in samples() {
        if let Message::Response(Response::Error { .. }) = envelope.message() {
            let wire = parse_line(&encode(&envelope).expect("encodes"));
            let keys: BTreeSet<&str> = wire["payload"]["error"]
                .as_object()
                .expect("an object")
                .keys()
                .map(String::as_str)
                .collect();
            assert!(keys.contains("code"), "{name}");
            assert!(
                keys.iter().all(
                    |key| ["code", "field", "kind", "reason", "supported", "max"].contains(key)
                ),
                "{name}: {keys:?}"
            );
            assert!(
                wire["payload"].as_object().expect("object").len() == 2,
                "{name}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// F: limits and framing
// ---------------------------------------------------------------------------

#[test]
fn a_frame_over_the_limit_is_rejected_before_it_is_parsed() {
    let max = u32::try_from(MAX_FRAME_LEN).expect("the limit fits in a u32");
    // Valid JSON, one byte too long.
    let error = rejected(&padded_to(MAX_FRAME_LEN + 1));
    assert_eq!(error.error(), &ProtocolError::FrameTooLarge { max });
    assert_eq!(
        error.id(),
        None,
        "an oversized frame is not read, so it has no id"
    );
    // Not JSON at all: the length still comes first.
    let error = rejected(&vec![0xff; MAX_FRAME_LEN + 1]);
    assert_eq!(error.error(), &ProtocolError::FrameTooLarge { max });
    let error = rejected(&vec![b'['; MAX_FRAME_LEN + 100]);
    assert_eq!(error.error(), &ProtocolError::FrameTooLarge { max });
    assert_eq!(
        parse_line(&encode(&error.into_response()).expect("encodes"))["payload"]["error"],
        json!({ "code": "frame_too_large", "max": max })
    );
}

#[test]
fn a_frame_of_exactly_the_limit_is_accepted() {
    let envelope = decode(&padded_to(MAX_FRAME_LEN)).expect("a frame at the limit");
    assert_eq!(envelope.message(), &Message::Request(Request::GetState));
}

#[test]
fn the_frame_limit_covers_the_largest_valid_blocklist() {
    // Lower bound of the largest valid UpsertBlocklist, from the model's own
    // maxima: its text values alone, with no keys, quotes or commas.
    let per_target = MAX_NAME_LEN + 4 * MAX_APP_ID_LEN;
    let text_only =
        MAX_TARGETS * per_target + MAX_EXCEPTIONS * MAX_PATTERN_LEN + MAX_ID_LEN + MAX_NAME_LEN;
    assert!(
        MAX_FRAME_LEN >= text_only,
        "MAX_FRAME_LEN = {MAX_FRAME_LEN} is below the {text_only} bytes of text a valid blocklist can hold"
    );
    // The encoded form, keys included, fits too (also in
    // a_blocklist_at_every_model_maximum_is_accepted).
    let valid = ValidatedBlocklist::new(largest_blocklist()).expect("valid");
    let frame = encode(&Envelope::new(
        mid("big"),
        Message::Request(Request::UpsertBlocklist { blocklist: valid }),
    ))
    .expect("encodes");
    assert!(frame.len() - 1 <= MAX_FRAME_LEN);
    // But not unbounded: the service never holds more than this per frame.
    let limit = std::hint::black_box(MAX_FRAME_LEN);
    assert!(limit <= 16 * 1024 * 1024, "{limit}");
}

#[test]
fn an_oversized_frame_is_refused_without_buffering_it() {
    let max = u32::try_from(MAX_FRAME_LEN).expect("fits");
    let mut reader = FrameReader::new();
    let chunk = vec![b'a'; 64 * 1024];
    let mut errors = Vec::new();
    for _ in 0..(MAX_FRAME_LEN / chunk.len() + 8) {
        for result in reader.push(&chunk) {
            errors.push(result.expect_err("no frame is complete"));
        }
        assert!(
            reader.buffered_len() <= MAX_FRAME_LEN,
            "buffered {} bytes",
            reader.buffered_len()
        );
    }
    assert_eq!(
        errors,
        vec![ProtocolError::FrameTooLarge { max }],
        "reported once"
    );

    // One huge slice with no newline is not copied whole either.
    let mut reader = FrameReader::new();
    let huge = vec![b'z'; MAX_FRAME_LEN * 2 + 10];
    let results = reader.push(&huge);
    assert_eq!(results, vec![Err(ProtocolError::FrameTooLarge { max })]);
    assert!(reader.buffered_len() <= MAX_FRAME_LEN);
}

#[test]
fn the_reader_recovers_after_an_oversized_frame() {
    let max = u32::try_from(MAX_FRAME_LEN).expect("fits");
    let good = bytes(&get_state_json());
    let mut reader = FrameReader::new();
    // The limit is hit before the newline arrives.
    assert_eq!(
        reader.push(&vec![b'x'; MAX_FRAME_LEN + 1]),
        vec![Err(ProtocolError::FrameTooLarge { max })]
    );
    // The rest of that line is dropped; the next line is read.
    assert_eq!(reader.push(b"more of the same line\n"), no_frames());
    let mut next = good.clone();
    next.push(b'\n');
    assert_eq!(reader.push(&next), vec![Ok(good.clone())]);
    assert_eq!(reader.buffered_len(), 0);
}

#[test]
fn an_oversized_frame_ending_in_the_same_chunk_is_reported_once() {
    let max = u32::try_from(MAX_FRAME_LEN).expect("fits");
    let good = bytes(&get_state_json());
    let mut stream = vec![b'x'; MAX_FRAME_LEN + 5];
    stream.push(b'\n');
    stream.extend_from_slice(&good);
    stream.push(b'\n');
    let mut reader = FrameReader::new();
    assert_eq!(
        reader.push(&stream),
        vec![Err(ProtocolError::FrameTooLarge { max }), Ok(good)]
    );
    // Exactly at the limit is a frame; one more byte is not.
    let mut at_limit = vec![b'y'; MAX_FRAME_LEN];
    at_limit.push(b'\n');
    let mut reader = FrameReader::new();
    assert_eq!(reader.push(&at_limit), vec![Ok(vec![b'y'; MAX_FRAME_LEN])]);
}

#[test]
fn the_reader_keeps_partial_frames() {
    let mut reader = FrameReader::new();
    assert_eq!(reader.push(b"abc"), no_frames());
    assert_eq!(reader.buffered_len(), 3);
    assert_eq!(reader.push(b"def\nxy"), vec![Ok(b"abcdef".to_vec())]);
    assert_eq!(reader.buffered_len(), 2);
    assert_eq!(reader.push(b"z\n"), vec![Ok(b"xyz".to_vec())]);
    assert_eq!(reader.buffered_len(), 0);
    // A frame cut in the middle of a UTF-8 character is whole once finished.
    let text = "caf\u{e9}".as_bytes().to_vec();
    let (first, second) = text.split_at_checked(4).expect("in range");
    assert_eq!(reader.push(first), no_frames());
    let mut rest = second.to_vec();
    rest.push(b'\n');
    assert_eq!(reader.push(&rest), vec![Ok(text)]);
    // Pushing nothing is fine.
    assert_eq!(reader.push(b""), no_frames());
    assert_eq!(FrameReader::default().buffered_len(), 0);
}

#[test]
fn the_reader_splits_several_frames_in_one_buffer() {
    let mut reader = FrameReader::new();
    assert_eq!(
        reader.push(b"one\ntwo\nthree\nfo"),
        vec![
            Ok(b"one".to_vec()),
            Ok(b"two".to_vec()),
            Ok(b"three".to_vec())
        ]
    );
    assert_eq!(reader.push(b"ur\n"), vec![Ok(b"four".to_vec())]);
}

#[test]
fn the_reader_gives_the_same_frames_for_any_chunking() {
    let mut stream = Vec::new();
    for (_, envelope) in samples() {
        stream.extend_from_slice(&encode(&envelope).expect("encodes"));
    }
    stream.extend_from_slice(b"\n\nnot json\n\xff\n");
    let whole = FrameReader::new().push(&stream);
    assert_eq!(whole.len(), samples().len() + 2);
    for size in [1, 2, 3, 7, 64, 1000] {
        let mut reader = FrameReader::new();
        let mut results = Vec::new();
        for chunk in stream.chunks(size) {
            results.extend(reader.push(chunk));
        }
        assert_eq!(results, whole, "chunks of {size}");
    }
    // And each frame is the encoded message, without its newline.
    for ((_, envelope), result) in samples().iter().zip(&whole) {
        let frame = result.as_ref().expect("a frame");
        assert_eq!(decode(frame).expect("decodes"), *envelope);
    }
}

#[test]
fn the_reader_skips_empty_frames() {
    let mut reader = FrameReader::new();
    assert_eq!(reader.push(b"\n"), no_frames());
    assert_eq!(reader.push(b"\n\n\nabc\n\n"), vec![Ok(b"abc".to_vec())]);
    assert_eq!(reader.buffered_len(), 0);
    // Decoding an empty frame directly is malformed, not a panic.
    assert_malformed(b"");
    assert_malformed(b"   ");
}

#[test]
fn the_reader_passes_nuls_and_invalid_utf8_to_the_decoder() {
    let mut reader = FrameReader::new();
    let results = reader.push(b"a\0b\n\xff\xfe\n\0\n");
    assert_eq!(
        results,
        vec![
            Ok(b"a\0b".to_vec()),
            Ok(b"\xff\xfe".to_vec()),
            Ok(b"\0".to_vec())
        ]
    );
    for result in results {
        assert_malformed(&result.expect("a frame"));
    }
    // Inside an otherwise valid frame.
    assert_malformed(
        b"{\"v\":1,\"id\":\"c1\",\"kind\":\"request\",\"payload\":{\"type\":\"get_state\"}}\0",
    );
    assert_malformed(
        b"{\"v\":1,\"id\":\"c\0\",\"kind\":\"request\",\"payload\":{\"type\":\"get_state\"}}",
    );
    assert_malformed(b"{\"v\":1,\"id\":\"c1\",\"kind\":\"request\",\"payload\":{\"type\":\"get_state\"},\"x\":\"\xff\"}");
    let mut with_nul = get_state_json();
    with_nul["payload"]["note"] = json!("a\u{0}b");
    // An escaped NUL in a string is legal JSON in an ignored field.
    decode(&bytes(&with_nul)).expect("an escaped NUL in an unknown field");
}

#[test]
fn malformed_input_is_a_typed_error_never_a_panic() {
    let deep_array = "[".repeat(100_000);
    let deep_object = r#"{"a":"#.repeat(50_000);
    let two_objects = format!("{}{}", get_state_json(), get_state_json());
    let trailing = format!("{} x", get_state_json());
    let inputs: Vec<&[u8]> = vec![
        b"",
        b"{",
        b"}",
        b"{}",
        b"[]",
        b"\"",
        b"{\"v\":1",
        b"{\"v\":1,}",
        b"nul\0l",
        b"\xef\xbb\xbf{}",
        b"\xc0\xaf",
        b"\x00\x00\x00\x00",
        deep_array.as_bytes(),
        deep_object.as_bytes(),
        two_objects.as_bytes(),
        trailing.as_bytes(),
        b"{\"v\":1,\"v\":2,\"id\":\"a\",\"kind\":\"request\",\"payload\":{\"type\":\"get_state\"}}x",
    ];
    for input in inputs {
        let error = decode(input).expect_err("rejected");
        assert!(
            matches!(
                error.error(),
                ProtocolError::Malformed | ProtocolError::UnsupportedVersion { .. }
            ),
            "{:?}",
            error.error()
        );
    }
    // Numbers at the edges of their types.
    for number in ["1e999", "4294967296", "18446744073709551616", "0.0000001"] {
        let frame = format!(
            "{{\"v\":1,\"id\":\"c1\",\"kind\":\"request\",\"payload\":{{\"type\":\"list_blocklists\",\"page\":{{\"offset\":{number},\"limit\":10}}}}}}"
        );
        assert_malformed(frame.as_bytes());
    }
}

#[test]
fn every_truncated_prefix_of_a_frame_is_malformed() {
    for (name, frame) in golden() {
        let full = bytes(&frame);
        for cut in 0..full.len() {
            let prefix = full.get(..cut).expect("in range");
            let error = decode(prefix).expect_err("a prefix is never a whole frame");
            assert_eq!(
                error.error(),
                &ProtocolError::Malformed,
                "{name} cut at {cut}"
            );
        }
    }
}

#[test]
fn random_input_never_panics() {
    // A fixed linear congruential generator, so a failure repeats.
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        seed >> 33
    };
    let alphabet: &[u8] = b"{}[]\":,\\ \nvidkindpayloadtype0123456789truefalsenull\x00\xff";
    let known: Vec<Vec<u8>> = golden().iter().map(|(_, frame)| bytes(frame)).collect();
    let mut reader = FrameReader::new();
    for round in 0_u64..4000 {
        let mut input = if round % 3 == 0 {
            // A known frame with a few bytes changed.
            let pick = usize::try_from(next()).expect("fits") % known.len();
            let mut frame = known.get(pick).expect("in range").clone();
            for _ in 0..3 {
                let at = usize::try_from(next()).expect("fits") % frame.len();
                let byte = *alphabet
                    .get(usize::try_from(next()).expect("fits") % alphabet.len())
                    .expect("in range");
                *frame.get_mut(at).expect("in range") = byte;
            }
            frame
        } else {
            let len = usize::try_from(next()).expect("fits") % 200;
            (0..len)
                .map(|_| {
                    *alphabet
                        .get(usize::try_from(next()).expect("fits") % alphabet.len())
                        .expect("in range")
                })
                .collect()
        };
        if let Ok(envelope) = decode(&input) {
            // Whatever is accepted is a message that survives a round trip.
            let again = encode(&envelope).expect("encodes");
            assert_eq!(decode(without_newline(&again)).expect("decodes"), envelope);
        }
        input.push(b'\n');
        for frame in reader.push(&input).into_iter().flatten() {
            let _ = decode(&frame);
        }
    }
}

#[test]
fn encoding_a_frame_over_the_limit_fails() {
    let max = u32::try_from(MAX_FRAME_LEN).expect("fits");
    let envelope = Envelope::new(
        mid("huge"),
        Message::Event(Event::TargetBlocked {
            event: BlockEvent {
                at_unix_ms: 1,
                kind: TargetKind::Site,
                target: long_text(MAX_FRAME_LEN),
            },
        }),
    );
    assert_eq!(encode(&envelope), Err(ProtocolError::FrameTooLarge { max }));
}

// ---------------------------------------------------------------------------
// H: housekeeping items of the task
// ---------------------------------------------------------------------------

/// The `path = "..."` values of the uncommented lines of a clippy.toml.
fn banned_paths(file: &str) -> Vec<&str> {
    file.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split("path = \"").nth(1)?.split('"').next())
        .collect()
}

fn bans(file: &str, suffix: &str) -> bool {
    banned_paths(file)
        .iter()
        .any(|path| *path == suffix || path.ends_with(&format!("::{suffix}")))
}

#[test]
fn split_off_sum_and_product_are_banned_in_all_three_clippy_files() {
    for (name, file) in [
        ("clippy.toml", ROOT_CLIPPY),
        ("crates/keitora-core/clippy.toml", CORE_CLIPPY),
        ("crates/keitora-ipc/clippy.toml", IPC_CLIPPY),
    ] {
        for suffix in [
            "Vec::split_off",
            "String::split_off",
            "iter::Sum::sum",
            "iter::Product::product",
            // Still there.
            "Iterator::sum",
            "Iterator::product",
            "slice::split_at",
            "str::split_at",
        ] {
            assert!(bans(file, suffix), "{name} does not ban {suffix}");
        }
    }
}

#[test]
fn bytes_methods_are_banned_if_the_crate_is_used() {
    let uses_bytes = IPC_CARGO_TOML
        .lines()
        .any(|line| line.trim_start().starts_with("bytes"));
    if !uses_bytes {
        return;
    }
    for (name, file) in [
        ("clippy.toml", ROOT_CLIPPY),
        ("crates/keitora-core/clippy.toml", CORE_CLIPPY),
        ("crates/keitora-ipc/clippy.toml", IPC_CLIPPY),
    ] {
        let paths = banned_paths(file);
        for method in [
            "split_to",
            "split_off",
            "advance",
            "get_u32",
            "get_u32_le",
            "get_u32_ne",
        ] {
            assert!(
                paths
                    .iter()
                    .any(|path| path.starts_with("bytes::")
                        && path.ends_with(&format!("::{method}"))),
                "{name} does not ban bytes::*::{method}"
            );
        }
    }
}

#[test]
fn the_crate_stays_pure() {
    assert!(IPC_LIB_RS.contains("#![forbid(unsafe_code)]"));
    assert!(
        IPC_CARGO_TOML.contains("keitora-core"),
        "messages carry core's types"
    );
    for line in IPC_CARGO_TOML
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
    {
        for forbidden in [
            "windows",
            "tokio",
            "async",
            "chrono",
            "keitora-sys",
            "keitora-service",
            "keitora-nmh",
            "rusqlite",
            "tracing",
            "reqwest",
            "hyper",
            "mio",
            "libc",
        ] {
            assert!(
                !line.contains(forbidden),
                "keitora-ipc must stay pure: {line}"
            );
        }
    }
}

#[test]
fn the_crate_docs_have_a_usage_example() {
    let docs: Vec<&str> = IPC_LIB_RS
        .lines()
        .take_while(|line| {
            line.starts_with("//!") || line.trim().is_empty() || line.starts_with("// ")
        })
        .collect();
    let fences: Vec<&&str> = docs
        .iter()
        .filter(|line| line.trim_start_matches("//!").trim().starts_with("```"))
        .collect();
    assert!(fences.len() >= 2, "the crate docs need a ``` example");
    for fence in fences {
        let info = fence
            .trim_start_matches("//!")
            .trim()
            .trim_start_matches('`');
        assert!(
            info.is_empty() || info == "rust",
            "the example must run as a doctest, not be ignored: {fence}"
        );
    }
    let text = docs.join("\n");
    assert!(text.contains("keitora_ipc::"), "the example uses the crate");
    assert!(
        text.contains("encode") && text.contains("decode"),
        "the example shows both directions"
    );
}

#[test]
fn the_placeholder_test_is_gone() {
    assert!(
        !IPC_LIB_RS.contains("gives_the_coverage_gate_a_line_to_measure"),
        "delete the placeholder test once the crate has real code"
    );
    assert!(
        !IPC_LIB_RS.contains("#[test]"),
        "unit tests of this crate live in a sibling tests.rs"
    );
}

#[test]
fn the_crate_left_the_placeholder_list() {
    let line = LAYOUT_RULES
        .lines()
        .find(|line| line.contains("PLACEHOLDER_CRATES = new Set("))
        .expect("scripts/unit-test-layout-rules.mjs defines PLACEHOLDER_CRATES");
    assert!(
        !line.contains("keitora-ipc"),
        "keitora-ipc has real code now: {line}"
    );
}
