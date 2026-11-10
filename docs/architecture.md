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
| `keitora-service` | Rust bin | Windows service, persistence, process monitor, pipe server, browser policies | `keitora-core`, `keitora-ipc`, `windows`, `windows-service`, `rusqlite`, `tokio`, `tracing` |
| `keitora-nmh` | Rust bin | Native messaging host: stdin/stdout ⇄ pipe | `keitora-ipc` |
| `apps/desktop` | Tauri app | UI; `src-tauri` forwards commands to the pipe and events to the frontend | `keitora-ipc`, `tauri`, `tauri-specta` |
| `extension` | TS | Applies site rules with `declarativeNetRequest`, shows the block page | — |

Dependencies only point downwards in this table. `keitora-core` never depends on anything platform-specific, which keeps it testable and reusable on Android and iOS later (via `uniffi`).

## IPC protocol (`keitora-ipc`)

- **Transport:** named pipe in message mode, one JSON object per line. The pipe's DACL allows the interactive user and `SYSTEM` only.
- **Envelope:** `{ "v": 1, "id": "<uuid>", "kind": "request" | "response" | "event", "payload": { … } }`.
- **Requests:** `GetState`, `StartSession { blocklist_id, preset, lock }`, `StopSession`, `PauseSession`, `SkipBreak`, `ListBlocklists`, `UpsertBlocklist`, `DeleteBlocklist`, `GetSettings`, `UpdateSettings`, `ListInstalledApps`, `GetHistory { range }`, `Subscribe`.
- **Events:** `StateChanged`, `PhaseChanged`, `TargetBlocked { target, kind, at }`, `SessionEnded`.
- **Typed errors:** `StrictLockActive { until }`, `Validation { field, message }`, `NotFound`, `UnsupportedVersion { supported }`.
- **Versioning:** additive changes keep `v`; breaking changes bump it. The service answers `UnsupportedVersion` instead of guessing.
- **Limits:** maximum message size and request rate per connection, enforced before deserializing.

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

Sites are platform-independent. Apps are logical entities with one identifier per platform ([ADR-0005](adr/0005-cross-platform-app-identity.md)); on iOS the identifier is an opaque, device-local token that is never synced. How sites and apps are matched against a blocklist is in [ADR-0009](adr/0009-textual-rule-matching.md). How a session keeps time is in [ADR-0010](adr/0010-session-time-as-active-time.md).

## Session lifecycle

1. The UI sends `StartSession`; the service validates it and persists a snapshot.
2. **Focus:** the process monitor enforces the blocklist; the extension receives site rules and redirects.
3. **Break:** blocking is lifted unless the blocklist sets `block_during_breaks`.
4. Every transition is persisted; after a reboot the service restores the session with the correct remaining time.
5. After the last cycle the service emits `SessionEnded` and writes the history entry.

## Storage

- SQLite at `%ProgramData%\Keitora\keitora.db`, migrations with `rusqlite_migration`.
- Directory ACL: write for `SYSTEM` only, read for users.
- Logs with `tracing` to `%ProgramData%\Keitora\logs`, rotated daily, no personal data beyond what the user configured.

## Open questions

- Event-driven process detection (ETW) instead of 500 ms polling — revisit if polling cost is measurable.
- Whether to block during breaks by default (current proposal: no, configurable per blocklist).
