# 0014 — Line-delimited JSON IPC protocol with validation at decode

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The service trusts nothing a client sends ([ADR-0001](0001-privileged-service-as-source-of-truth.md), `docs/threat-model.md`). `keitora-ipc` defines the messages and runs inside the service as `SYSTEM`, so its parser is attack surface: a frame can be huge, deeply nested, full of unknown fields or not UTF-8, and a panic is a denial of service. Earlier reviews found that serde's internally tagged enums and `flatten` copy every field, unknown ones included, into a buffer (a 25x memory amplification in T-1.1), and that a framing reader must not buffer a line it has already refused.

## Decision

- **Framing:** one frame per line, compact JSON ending in `\n`. `FrameReader` cuts a byte stream into frames, skips empty lines, never holds more than `MAX_FRAME_LEN` bytes, and after a line passes the limit reports `FrameTooLarge` once and drops the rest of that line without storing it. NULs and invalid UTF-8 are passed to `decode`, which rejects them. `encode` enforces the same limit.
- **Frame limit:** `MAX_FRAME_LEN` is 8 MiB. The largest valid blocklist is about 2.3 MB of JSON (500 targets with the maximum name and four identifiers, 200 exceptions), up to about 4.6 MB if every character is escaped; 8 MiB holds it and equals the largest value the store reads (ADR-0013). The length is checked on the raw bytes before any parsing. Per-connection rate limits belong to the pipe server (T-2.4).
- **Envelope:** `{ "v", "id", "kind", "payload": { "type", …fields } }`. The decoder reads the envelope with fields kept as borrowed slices of the frame (`serde_json` `RawValue`), so the version is checked before the payload is looked at, and nothing is buffered. The payload's `type` is read by a struct that skips everything else, then the fields of that one message by a plain struct. No `#[serde(tag)]` or `flatten` is used to deserialize; `tag` is only used to serialize, where nothing is buffered.
- **Versioning:** `v` is one whole number, the major version, `1`. A whole number other than 1 is `unsupported_version { supported: [1] }`; anything else (missing, text, fraction, negative) is `malformed`. Additive changes keep `v`; breaking ones bump it.
- **Unknown fields are ignored** at every level, so a newer peer's additions don't break an older one, as `keitora-core` already does. An unknown message `type` or `kind` is `malformed`: the receiver answers with the frame's id (or `"unknown"`) and the stream goes on. Optional fields may be absent or `null`; required ones, such as `lock`, never default, so a missing lock can't become an unlocked session.
- **Validation at decode:** wrong structure or types is `malformed`; a well-formed value out of range is `validation { field, kind }` with the path of the field from the payload. Blocklists travel as `ValidatedBlocklist`, the only way to put one in a message; presets, locks, ids, page limits (1 to 100) and stop text (1024 bytes) are checked too. Lists are always paginated.
- **No time in requests:** no request carries a time, instant or duration, and `GetHistory` takes a named range. There is no request to move a clock, end, shorten or re-time a session, or check a URL.
- **Typed errors** are `strict_session_locked`, `validation`, `not_found`, `unsupported_version`, `frame_too_large`, `malformed`, `invalid_state` and `internal`. They carry only a stable code and the fields named in each variant, never client text or parser messages; `field` is chosen by this crate. `DecodeError` keeps the id only if it is valid.
- `PhaseChanged` is dropped: `StateChanged` carries the phase.
- Unsafe-free, no OS, async or time dependencies; `Vec::split_off`, `String::split_off` and `Sum`/`Product` are banned by clippy.

## Alternatives considered

- **serde `tag`/`flatten` or `untagged` enums**: shortest code, but buffer unknown fields (above).
- **Parsing to `serde_json::Value` first**: simple, but a tree costs many times the frame's size.
- **Length-prefixed frames**: no scanning for newlines, but harder to debug and the architecture already chose line-delimited JSON for the native messaging bridge.
- **A smaller limit with a chunked upload for blocklists**: more protocol for no gain at this size.
- **Clamping out-of-range values**: hides client bugs and could change a session's meaning; values are rejected.

## Consequences

- A reader holds at most 8 MiB per connection and a decode may use several times that; the pipe server must bound connections.
- Clients that only decode responses also validate them (a response with an invalid blocklist is rejected).
- New messages are additive; `Request`, `Response`, `Event`, `ProtocolError` and `ValidationKind` are `#[non_exhaustive]`.
- TypeScript types for the UI (`tauri-specta`) will be generated later from these types.
