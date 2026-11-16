# Architecture

This document explains how Keitora is put together and why. Decisions are recorded as ADRs in [`docs/adr/`](adr/); security assumptions are in [threat-model.md](threat-model.md).

## Overview

```
┌──────────────────────┐  Tauri commands  ┌──────────────────────┐
│ Vue 3 UI             │◄────────────────►│ src-tauri (client)   │
└──────────────────────┘                  └──────────┬───────────┘
                                                     │ named pipe \\.\pipe\keitora
┌──────────────────────┐ native messaging ┌──────────▼───────────┐
│ MV3 extension        │◄────────────────►│ keitora-nmh.exe      │──┐
│ (Chrome / Edge)      │  stdin / stdout  └──────────────────────┘  │ named pipe
└──────────────────────┘                                            ▼
                                        ┌──────────────────────────────────────┐
                                        │ keitora-service  (LocalSystem)        │
                                        │  ├─ keitora-core: engine + rules      │
                                        │  ├─ process monitor                   │
                                        │  ├─ browser policies (HKLM)           │
                                        │  └─ SQLite  %ProgramData%\Keitora     │
                                        └──────────────────────────────────────┘
```

**Guiding principle:** the service is the only source of truth. The UI and the extension are untrusted clients — they request, the service decides ([ADR-0001](adr/0001-privileged-service-as-source-of-truth.md)).

## Crates and packages

| Unit | Kind | Responsibility | May depend on |
|---|---|---|---|
| `keitora-core` | Rust lib | Domain model, pomodoro state machine, site/app rule matching, strict-mode policy | `serde`, `thiserror`, `chrono` |
| `keitora-ipc` | Rust lib | Protocol types, versioning, line-delimited JSON framing | `keitora-core`, `serde` |
| `keitora-sys` | Rust lib | Safe wrappers around Win32 calls, one per call; the only crate with `unsafe` code | `windows-sys` |
| `keitora-service` | Rust bin + lib | Windows service, persistence, process monitor, pipe server, browser policies | `keitora-core`, `keitora-ipc`, `keitora-sys`, `windows-service`, `rusqlite`, `tokio`, `tracing` |
| `keitora-nmh` | Rust bin | Native messaging host: stdin/stdout ⇄ pipe | `keitora-ipc`; `keitora-sys` when it needs a Win32 call |
| `apps/desktop` | Tauri app | UI; `src-tauri` forwards commands to the pipe and events to the frontend | `keitora-ipc`, `tauri`, `tauri-specta`; `keitora-sys` when it needs a Win32 call |
| `extension` | TS | Applies site rules with `declarativeNetRequest`, shows the block page | — |

Dependencies only point downwards in this table. `keitora-core` never depends on anything platform-specific, which keeps it testable and reusable on Android and iOS later (via `uniffi`).

Every crate but `keitora-sys` has `#![forbid(unsafe_code)]`: a Win32 call that the standard library or a vetted crate doesn't offer gets a safe wrapper in `keitora-sys` ([ADR-0012](adr/0012-isolate-unsafe-code-in-keitora-sys.md)).

## IPC protocol (`keitora-ipc`)

- **Transport:** named pipe in byte mode, one JSON object per line (frames are lines of text and may be longer than a pipe message, so message mode is not used). The pipe's DACL allows `SYSTEM`, the interactive user and the account the service runs as, and no one else. The interactive user gets `0x12019b`: everything a client needs, without `FILE_CREATE_PIPE_INSTANCE`, so no process of that user can serve the pipe's name. An installed service runs as `SYSTEM`, so its DACL grants that right to no user; under `--console` the service's own ACE is the interactive user's, which keeps the exposure in development only ([threat-model.md](threat-model.md)). Clients open their end with that same mask, through `keitora_sys::named_pipe::open_client`. Remote clients are rejected.
- **Framing:** one frame per line: compact JSON ending in `
`, at most `MAX_FRAME_LEN` (8 MiB) bytes without it. The length is checked on the raw bytes, before parsing; a longer line is refused and the reader skips the rest of it without storing it ([ADR-0014](adr/0014-ipc-protocol-wire-format.md)).
- **Envelope:** `{ "v": 1, "id": "<id>", "kind": "request" | "response" | "event", "payload": { "type": "<snake_case name>", … } }`. The id is 1 to 64 bytes of ASCII letters, digits and `-`; a response carries the id of its request. The payload is flat: its fields sit next to `type`.
- **Requests:** `GetState`, `StartSession { blocklist_id, preset, lock }`, `StopSession { text }`, `PauseSession`, `ResumeSession`, `SkipBreak`, `ListBlocklists { page }`, `GetBlocklist { id }`, `UpsertBlocklist { blocklist }`, `DeleteBlocklist { id }`, `GetSettings`, `UpdateSettings { settings }`, `ListInstalledApps { page }`, `GetHistory { range, page }`, `GetBlockEvents { page }`, `Subscribe`. Lists are paginated (`page` is `{ offset, limit }`, limit 1 to 100, and responses carry `next_offset`). No request carries a time, and none can end, shorten or re-time a session.
- **Responses:** `Ok`, `Subscribed`, `State`, `Blocklists`, `Blocklist`, `Settings`, `InstalledApps`, `History`, `BlockEvents`, `Error { error }`.
- **Events:** `StateChanged { state }` (it also reports phase changes), `TargetBlocked { event }`, `BlocklistChanged { id }`, `SessionEnded { outcome }`.
- **Typed errors** (`{ "code": …}`): `strict_session_locked { reason }`, `validation { field, kind }`, `not_found`, `unsupported_version { supported }`, `frame_too_large { max }`, `malformed`, `invalid_state`, `internal`. They never echo the client's text nor internal details.
- **Versioning:** `v` is one whole number, the major version. Additive changes keep it (unknown fields are ignored at every level); breaking changes bump it. A different `v` is answered with `unsupported_version` before the payload is read; an unknown `type` or `kind` is `malformed`, and the stream goes on.
- **Validation:** the decoder validates every request (blocklists, presets, locks, ids, pages, typed text) and rejects what is out of range with `validation`, so an invalid value never reaches the service. Request rate per connection is limited by the pipe server (T-2.4).
- **Pipe server limits (T-2.4):** at most 64 simultaneous connections (one beyond is closed at once); 50 requests a second sustained per connection with a burst of 100 (a faster client is slowed, not failed); at most 16 MiB (two maximum frames) waiting to be written to one connection, beyond which the connection is closed.

TypeScript types for the UI are generated from these Rust types (`tauri-specta`), so there is one definition of every message.

### Why native messaging instead of a local WebSocket

A local port can be reached by any process and needs its own authentication. A native messaging host is launched by the browser itself, only for extension IDs listed in its manifest, and it reaches the service over the DACL-protected pipe ([ADR-0004](adr/0004-native-messaging-bridge.md)).

## Data model (summary)

```text
Blocklist      { id, name, color, targets: [Target], exceptions: [SitePattern], block_during_breaks: bool }
Target         = Site { pattern }                    // "youtube.com", "*.reddit.com", "youtube.com/shorts"
               | App  { name, ids: AppIdentifiers }
AppIdentifiers { windows_exe?, windows_path?, android_package?, ios_token? }
PomodoroPreset { focus_min, short_break_min, long_break_min, cycles }
Session        { id, blocklist_id, preset, lock: LockKind, started_at, phase, cycle, remaining }
LockKind       = None | UntilTimerEnds | RandomText { len }
```

Only `UntilTimerEnds` makes a session strict. `RandomText` adds friction: typing the text stops the session, so it isn't a guarantee against a determined user or a script.

Sites are platform-independent. Apps are logical entities with one identifier per platform ([ADR-0005](adr/0005-cross-platform-app-identity.md)); on iOS the identifier is an opaque, device-local token that is never synced. How sites and apps are matched against a blocklist is in [ADR-0009](adr/0009-textual-rule-matching.md). How a session keeps time is in [ADR-0010](adr/0010-session-time-as-active-time.md), and how it is persisted and restored in [ADR-0011](adr/0011-validated-engine-snapshot.md).

## Session lifecycle

1. The UI sends `StartSession`; the service validates it and persists a snapshot.
2. **Focus:** the process monitor enforces the blocklist; the extension receives site rules and redirects.
3. **Break:** blocking is lifted unless the blocklist sets `block_during_breaks`.
4. Every transition is persisted; after a reboot the service restores the session with the correct remaining time.
5. After the last cycle the service emits `SessionEnded` and writes the history entry.

## Service binary

- `keitora-service` with no arguments runs as a Windows service, started by the Service Control Manager. Development builds also have `keitora-service --console [--data-dir <DIR>]`, which runs it in a terminal until standard input closes or Ctrl+C; release builds reject it.
- The first thing it does is restrict DLL loading to `System32` (`SetDefaultDllDirectories`); if that fails, it refuses to run.
- Its one clock reads the unbiased interrupt time, so time asleep or hibernated doesn't count toward a session ([ADR-0010](adr/0010-session-time-as-active-time.md)).

## Storage

- SQLite at `%ProgramData%\Keitora\keitora.db`, migrations with `rusqlite_migration` (`keitora_service::store`): blocklists, settings, session history, block events and the engine snapshot. Blocklists are validated when stored and again when read; stored data that fails a check is an error, never replaced by a default. A damaged file, one from a newer schema, or one whose schema, root pages or format aren't exactly the store's, is refused and never reset. No SQL stored in the file runs (triggers and views are off), and the bundled SQLite is built without WAL (`.cargo/config.toml`), so a planted `keitora.db-wal` is never read; a build whose SQLite has WAL refuses to open the store.
- The engine snapshot is overwritten in place in a fixed-size slot, with a persistent rollback journal and every commit flushed, so checkpoints need no new disk space and a full disk can't erase a strict session ([ADR-0013](adr/0013-store-snapshot-in-place-and-never-reset.md)).
- Directory ACL: write for `SYSTEM` and Administrators only, read for users: a protected DACL that inherits nothing from `C:\ProgramData`, which lets every user create files in its subfolders. Until T-5.3 sets it, any standard user can plant files in `Keitora` and `logs` ([threat-model.md](threat-model.md)).
- Logs with `tracing` to `%ProgramData%\Keitora\logs`, one file per day (`keitora.YYYY-MM-DD.log`, UTC), the last 14 kept, never blocklists or other user data. If the folder can't be used, logs go to stderr and the service keeps running.

## Open questions

- Event-driven process detection (ETW) instead of 500 ms polling — revisit if polling cost is measurable.
- Whether to block during breaks by default (current proposal: no, configurable per blocklist).
