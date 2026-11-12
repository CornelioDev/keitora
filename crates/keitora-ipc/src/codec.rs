// SPDX-License-Identifier: MIT OR Apache-2.0

//! Turning an [`Envelope`] into a frame and a frame into an [`Envelope`].

use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

use crate::error::{DecodeError, ProtocolError};
use crate::frame::{MAX_FRAME_LEN, frame_too_large};
use crate::message::{Envelope, Event, Message, PROTOCOL_VERSION, Request, Response};
use crate::raw;
use crate::types::MessageId;

/// What `payload` holds, to serialize it without copying.
enum PayloadRef<'a> {
    Request(&'a Request),
    Response(&'a Response),
    Event(&'a Event),
}

impl Serialize for PayloadRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Request(request) => request.serialize(serializer),
            Self::Response(response) => response.serialize(serializer),
            Self::Event(event) => event.serialize(serializer),
        }
    }
}

/// An envelope as written.
#[derive(Serialize)]
struct WireOut<'a> {
    v: u32,
    id: &'a MessageId,
    kind: &'static str,
    payload: PayloadRef<'a>,
}

/// Writes an envelope as a frame: one line of compact JSON ending in `\n`.
///
/// Every field is written; an empty option is `null`.
///
/// # Errors
///
/// [`ProtocolError::FrameTooLarge`] if the line, not counting its `\n`, would
/// be longer than [`MAX_FRAME_LEN`]: the limit is the same for what is sent
/// and what is received.
pub fn encode(envelope: &Envelope) -> Result<Vec<u8>, ProtocolError> {
    let (kind, payload) = match envelope.message() {
        Message::Request(request) => ("request", PayloadRef::Request(request)),
        Message::Response(response) => ("response", PayloadRef::Response(response)),
        Message::Event(event) => ("event", PayloadRef::Event(event)),
    };
    let wire = WireOut {
        v: PROTOCOL_VERSION,
        id: envelope.id(),
        kind,
        payload,
    };
    let mut frame = serde_json::to_vec(&wire).map_err(|_| ProtocolError::Internal)?;
    if frame.len() > MAX_FRAME_LEN {
        return Err(frame_too_large());
    }
    frame.push(b'\n');
    Ok(frame)
}

/// The envelope of a frame, with its payload still as text. Each field is a
/// slice of the frame, so nothing is copied or buffered, whatever the payload
/// holds; a `null` counts as missing.
#[derive(Deserialize)]
struct Header<'a> {
    #[serde(borrow)]
    v: Option<&'a RawValue>,
    #[serde(borrow)]
    id: Option<&'a RawValue>,
    #[serde(borrow)]
    kind: Option<&'a RawValue>,
    #[serde(borrow)]
    payload: Option<&'a RawValue>,
}

/// The id in a header, if it is valid. An invalid one is dropped, never kept.
fn read_id(raw: Option<&RawValue>) -> Option<MessageId> {
    let text: String = serde_json::from_str(raw?.get()).ok()?;
    MessageId::new(&text).ok()
}

/// Checks the version in a header: only [`PROTOCOL_VERSION`] passes.
fn check_version(raw: Option<&RawValue>) -> Result<(), ProtocolError> {
    let text = raw.ok_or(ProtocolError::Malformed)?.get();
    if text == PROTOCOL_VERSION.to_string() {
        Ok(())
    } else if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
        // A whole number other than 1, however large.
        Err(ProtocolError::UnsupportedVersion {
            supported: vec![PROTOCOL_VERSION],
        })
    } else {
        Err(ProtocolError::Malformed)
    }
}

/// Reads a frame, without its `\n`.
///
/// The length is checked first, on the raw bytes, before anything is parsed.
/// Then the version is checked before the payload is looked at. Fields this
/// crate doesn't know are ignored at every level, so a newer peer's additions
/// don't break an older one; an unknown message `type` or `kind` is
/// [`ProtocolError::Malformed`].
///
/// # Errors
///
/// A [`DecodeError`] with the reason (see [`ProtocolError`]) and the frame's
/// id, if it was read and valid, so that the receiver can answer with
/// [`DecodeError::into_response`] and go on with the next frame.
pub fn decode(frame: &[u8]) -> Result<Envelope, DecodeError> {
    if frame.len() > MAX_FRAME_LEN {
        return Err(DecodeError::new(frame_too_large(), None));
    }
    let text =
        std::str::from_utf8(frame).map_err(|_| DecodeError::new(ProtocolError::Malformed, None))?;
    let header: Header<'_> =
        serde_json::from_str(text).map_err(|_| DecodeError::new(ProtocolError::Malformed, None))?;
    let id = read_id(header.id);
    check_version(header.v).map_err(|error| DecodeError::new(error, id.clone()))?;
    // A frame without a usable id can't be answered under it.
    let id = id.ok_or_else(|| DecodeError::new(ProtocolError::Malformed, None))?;
    let fail = |error: ProtocolError| DecodeError::new(error, Some(id.clone()));
    let kind: String = header
        .kind
        .and_then(|raw| serde_json::from_str(raw.get()).ok())
        .ok_or_else(|| fail(ProtocolError::Malformed))?;
    let payload = header
        .payload
        .ok_or_else(|| fail(ProtocolError::Malformed))?
        .get();
    let message = match kind.as_str() {
        "request" => raw::request(payload).map(Message::Request),
        "response" => raw::response(payload).map(Message::Response),
        "event" => raw::event(payload).map(Message::Event),
        _ => Err(ProtocolError::Malformed),
    }
    .map_err(fail)?;
    Ok(Envelope::new(id, message))
}

#[cfg(test)]
mod tests;
