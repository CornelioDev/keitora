// SPDX-License-Identifier: MIT OR Apache-2.0

// The one seam between the frontend and Tauri: typed wrappers around
// `invoke` and the service's event stream. Components never import
// `@tauri-apps/api` themselves; they use the composables, which use this.
//
// Command names are the snake_case form of the requests in
// docs/architecture.md, and argument names are the camelCase form of the
// request fields (Tauri's convention). The Rust side (T-3.1) must register
// the same names. A command that fails rejects with the service's error
// (`{ code, ... }`, see `ServiceError`), passed on untouched. Nothing here
// uses the network, and no request carries a time: the service owns the clock.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type {
  Blocklist,
  BlocklistSummary,
  Page,
  PageRequest,
  ServiceEvent,
  SessionState,
  Settings,
  StartSessionRequest,
} from "./types";

/** The Tauri event under which the Rust side forwards service events. */
export const SERVICE_EVENT = "service-event";

/** The state of the session. */
export function getState(): Promise<SessionState> {
  return invoke<SessionState>("get_state");
}

/** Starts a session with what the user chose. */
export function startSession(request: StartSessionRequest): Promise<void> {
  return invoke<void>("start_session", {
    blocklistId: request.blocklist_id,
    preset: request.preset,
    lock: request.lock,
  });
}

/**
 * Asks to stop the session. `text` is what the user typed for a
 * `random_text` lock. A strict session refuses.
 */
export function stopSession(text?: string): Promise<void> {
  return invoke<void>("stop_session", { text: text ?? null });
}

/** Pauses the session; a strict one refuses. */
export function pauseSession(): Promise<void> {
  return invoke<void>("pause_session");
}

/** Resumes a paused session. */
export function resumeSession(): Promise<void> {
  return invoke<void>("resume_session");
}

/** Ends the current break early. */
export function skipBreak(): Promise<void> {
  return invoke<void>("skip_break");
}

/** One page of the blocklists. */
export function listBlocklists(
  page: PageRequest,
): Promise<Page<BlocklistSummary>> {
  return invoke<Page<BlocklistSummary>>("list_blocklists", { page });
}

/** One blocklist, with its targets. */
export function getBlocklist(id: string): Promise<Blocklist> {
  return invoke<Blocklist>("get_blocklist", { id });
}

/** Creates a blocklist or replaces the one with the same id. */
export function upsertBlocklist(blocklist: Blocklist): Promise<void> {
  return invoke<void>("upsert_blocklist", { blocklist });
}

/** Deletes a blocklist. */
export function deleteBlocklist(id: string): Promise<void> {
  return invoke<void>("delete_blocklist", { id });
}

/** The user's settings. */
export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/** Replaces the user's settings. */
export function updateSettings(settings: Settings): Promise<void> {
  return invoke<void>("update_settings", { settings });
}

/**
 * Calls `handler` with every event the service sends, until the returned
 * function is called. Each call registers its own listener.
 */
export async function onServiceEvent(
  handler: (event: ServiceEvent) => void,
): Promise<() => void> {
  return listen<ServiceEvent>(SERVICE_EVENT, (event) => {
    handler(event.payload);
  });
}
