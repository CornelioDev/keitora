// SPDX-License-Identifier: MIT OR Apache-2.0

// The shapes of docs/architecture.md ("IPC protocol") as the frontend sees
// them: the JSON the Rust types of `keitora-ipc` and `keitora-core` produce.
// They are written by hand until the generated bindings of T-3.1 replace
// them; keep the field names identical to the Rust ones.

/** The phase of the session; `idle` when there is none. */
export type Phase = "idle" | "focus" | "short_break" | "long_break";

/** How firmly a session is locked against being cut short. */
export type LockKind =
  | { kind: "none" }
  | { kind: "until_timer_ends" }
  | { kind: "random_text"; len: number };

/** The durations and cycles of a session. */
export interface PomodoroPreset {
  focus_min: number;
  short_break_min: number;
  long_break_min: number;
  cycles: number;
}

/** What the service reports about the session. */
export interface SessionState {
  phase: Phase;
  cycle: number;
  remaining_ms: number;
  paused: boolean;
  lock: LockKind | null;
  blocklist_id: string | null;
}

/** The user's settings. */
export interface Settings {
  default_preset: PomodoroPreset;
}

/** Which part of a list a request asks for (limit 1 to 100). */
export interface PageRequest {
  offset: number;
  limit: number;
}

/** A page of a list; `next_offset` is `null` on the last page. */
export interface Page<T> {
  items: T[];
  next_offset: number | null;
}

/** A blocklist as listed, without its targets. */
export interface BlocklistSummary {
  id: string;
  name: string;
  color: string;
}

/** The identifiers of an application on each platform. */
export interface AppIdentifiers {
  windows_exe?: string;
  windows_path?: string;
  android_package?: string;
  ios_token?: string;
}

/**
 * Something a blocklist blocks: a site (`kind: "site"` with a `pattern`) or
 * an application (`kind: "app"` with a `name` and `ids`). `kind` is a plain
 * string and the other fields are optional because the service validates
 * every target; the generated bindings of T-3.1 will narrow this.
 */
export interface Target {
  kind: string;
  pattern?: string;
  name?: string;
  ids?: AppIdentifiers;
}

/** A blocklist with its targets. */
export interface Blocklist extends BlocklistSummary {
  targets: Target[];
  exceptions: string[];
  block_during_breaks: boolean;
}

/** What the user chooses to start a session. It never carries a time. */
export interface StartSessionRequest {
  blocklist_id: string;
  preset: PomodoroPreset;
  lock: LockKind;
}

/** How a session ended. */
export type Outcome = "completed" | "stopped";

/** What the service tells the UI, flat as on the wire. */
export type ServiceEvent =
  | { type: "state_changed"; state: SessionState }
  | { type: "target_blocked"; event: unknown }
  | { type: "blocklist_changed"; id: string }
  | { type: "session_ended"; outcome: Outcome };

/**
 * An error from the service, as `keitora-ipc`'s `ProtocolError` serializes
 * it: `code` tells `strict_session_locked` from `not_found` and the rest.
 */
export interface ServiceError {
  code:
    | "strict_session_locked"
    | "validation"
    | "not_found"
    | "unsupported_version"
    | "frame_too_large"
    | "malformed"
    | "invalid_state"
    | "internal";
  [detail: string]: unknown;
}
