// SPDX-License-Identifier: MIT OR Apache-2.0

// A stand-in for the Tauri boundary (`invoke` and event listeners), used by
// the T-3.2 acceptance tests. T-3.1 (the real IPC client) does not exist yet,
// so these tests mock `@tauri-apps/api/core` and `@tauri-apps/api/event` and
// never rely on generated bindings or on the exact name of a Tauri command.
//
// A command is matched by its name with case and `_`/`-` ignored, as a suffix:
// `get_state`, `getState` and `ipc_get_state` all answer to "getstate". The
// names follow the requests of docs/architecture.md (GetState, ListBlocklists,
// GetBlocklist, UpsertBlocklist, DeleteBlocklist, GetSettings, UpdateSettings,
// StartSession, StopSession, PauseSession, ResumeSession, SkipBreak).

import { vi } from "vitest";

export type Handler = (args: Record<string, unknown> | undefined) => unknown;

export interface InvokeCall {
  command: string;
  args: Record<string, unknown> | undefined;
}

function normalize(name: string): string {
  return name.toLowerCase().replace(/[^a-z]/g, "");
}

export class TauriFake {
  readonly calls: InvokeCall[] = [];
  readonly unlisten = vi.fn();
  private handlers = new Map<string, Handler>();
  private listeners: ((event: {
    event: string;
    id: number;
    payload: unknown;
  }) => void)[] = [];

  /** Registers the answer to a command, e.g. `on("getstate", () => state)`. */
  on(command: string, handler: Handler): void {
    this.handlers.set(normalize(command), handler);
  }

  /** How many times a command was invoked. */
  count(command: string): number {
    const wanted = normalize(command);
    return this.calls.filter((c) => normalize(c.command).endsWith(wanted))
      .length;
  }

  /** The arguments of the commands invoked under a name, in order. */
  argsOf(command: string): (Record<string, unknown> | undefined)[] {
    const wanted = normalize(command);
    return this.calls
      .filter((c) => normalize(c.command).endsWith(wanted))
      .map((c) => c.args);
  }

  /** Forgets handlers, calls and listeners between tests. */
  reset(): void {
    this.calls.length = 0;
    this.handlers.clear();
    this.listeners = [];
    this.invoke.mockClear();
    this.listen.mockClear();
    this.unlisten.mockClear();
  }

  get listenerCount(): number {
    return this.listeners.length;
  }

  readonly invoke = vi.fn(
    async (command: string, args?: Record<string, unknown>) => {
      this.calls.push({ command, args });
      const wanted = normalize(command);
      const key = [...this.handlers.keys()]
        .filter((k) => wanted.endsWith(k))
        .sort((a, b) => b.length - a.length)[0];
      if (key === undefined) {
        throw new Error(`TauriFake: no handler for command "${command}"`);
      }
      const handler = this.handlers.get(key);
      return handler?.(args);
    },
  );

  readonly listen = vi.fn(
    async (
      eventName: string,
      callback: (event: {
        event: string;
        id: number;
        payload: unknown;
      }) => void,
    ) => {
      void eventName;
      this.listeners.push(callback);
      return this.unlisten;
    },
  );

  /**
   * Delivers a service event (`{ type: "state_changed", ... }`, the flat
   * payload of docs/architecture.md) to every listener, whatever event name
   * it registered under.
   */
  emit(payload: Record<string, unknown>): void {
    for (const listener of [...this.listeners]) {
      listener({ event: "service-event", id: 1, payload });
    }
  }
}

/** The one fake the mocked Tauri modules use. */
export const tauri = new TauriFake();

export const IDLE_STATE = {
  phase: "idle",
  cycle: 0,
  remaining_ms: 0,
  paused: false,
  lock: null,
  blocklist_id: null,
};

export const FOCUS_STATE = {
  phase: "focus",
  cycle: 1,
  remaining_ms: 1_500_000,
  paused: false,
  lock: { kind: "until_timer_ends" },
  blocklist_id: "work",
};

export const PRESET = {
  focus_min: 25,
  short_break_min: 5,
  long_break_min: 15,
  cycles: 4,
};

export const SETTINGS = { default_preset: PRESET };

export const WORK_LIST = { id: "work", name: "Work", color: "#3366ff" };
export const SOCIAL_LIST = { id: "social", name: "Social", color: "#ff3366" };

/** Wait until `check` stops throwing, polling microtasks and timers. */
export async function eventually(check: () => void): Promise<void> {
  let last: unknown;
  for (let i = 0; i < 100; i += 1) {
    try {
      check();
      return;
    } catch (error) {
      last = error;
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
  }
  throw last;
}
