// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.2, `services/ipc.ts` (the single seam to Tauri).
//
// The roadmap has no acceptance line for T-3.2; these criteria are derived
// from its text:
//  - AC-1 typed wrappers around Tauri `invoke`: every request of
//    docs/architecture.md has a wrapper that invokes once and returns what
//    the command returned
//      getState_returns_what_the_command_returns
//      listBlocklists_sends_the_page_and_returns_the_page
//      getBlocklist_sends_the_id
//      upsertBlocklist_and_deleteBlocklist_send_their_arguments
//      settings_wrappers_read_and_replace_the_settings
//      session_control_wrappers_invoke_one_command_each
//      stopSession_passes_the_typed_text_for_a_random_text_lock
//  - AC-2 startSession sends only what the user chose (blocklist, preset,
//    lock); the UI never supplies a time
//      startSession_sends_the_blocklist_preset_and_lock_and_no_time
//  - AC-3 service errors reach the caller intact, so the UI can tell
//    `strict_session_locked` from `not_found`
//      a_service_error_reaches_the_caller_with_its_code
//      a_not_found_error_keeps_a_different_code_from_a_locked_one
//  - AC-4 `onServiceEvent` wraps the Tauri event listener
//      handlers_receive_the_flat_event_payload
//      the_returned_function_stops_listening
//      two_handlers_can_listen_at_once_and_each_is_called
//  - AC-5 no network (no telemetry rule)
//      wrappers_never_call_fetch

import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  FOCUS_STATE,
  IDLE_STATE,
  PRESET,
  SETTINGS,
  WORK_LIST,
  tauri,
} from "./support/t3-2-tauri-fake";

vi.mock("@tauri-apps/api/core", async () => {
  const { tauri } = await import("./support/t3-2-tauri-fake");
  return { invoke: tauri.invoke };
});
vi.mock("@tauri-apps/api/event", async () => {
  const { tauri } = await import("./support/t3-2-tauri-fake");
  return { listen: tauri.listen };
});

import * as ipc from "@/services/ipc";

beforeEach(() => {
  tauri.reset();
});

describe("services/ipc wrappers", () => {
  it("getState_returns_what_the_command_returns", async () => {
    tauri.on("getstate", () => FOCUS_STATE);

    await expect(ipc.getState()).resolves.toEqual(FOCUS_STATE);
    expect(tauri.count("getstate")).toBe(1);
  });

  it("listBlocklists_sends_the_page_and_returns_the_page", async () => {
    const page = { items: [WORK_LIST], next_offset: null };
    tauri.on("listblocklists", () => page);

    await expect(ipc.listBlocklists({ offset: 0, limit: 50 })).resolves.toEqual(
      page,
    );

    expect(JSON.stringify(tauri.argsOf("listblocklists"))).toContain("50");
    expect(tauri.count("listblocklists")).toBe(1);
  });

  it("getBlocklist_sends_the_id", async () => {
    const full = { ...WORK_LIST, targets: [], exceptions: [] };
    tauri.on("getblocklist", () => full);

    await expect(ipc.getBlocklist("work")).resolves.toEqual(full);
    expect(JSON.stringify(tauri.argsOf("getblocklist"))).toContain("work");
  });

  it("upsertBlocklist_and_deleteBlocklist_send_their_arguments", async () => {
    tauri.on("upsertblocklist", () => undefined);
    tauri.on("deleteblocklist", () => undefined);
    const blocklist = {
      ...WORK_LIST,
      targets: [{ kind: "site", pattern: "example.com" }],
      exceptions: [],
      block_during_breaks: false,
    };

    await ipc.upsertBlocklist(blocklist);
    await ipc.deleteBlocklist("work");

    expect(JSON.stringify(tauri.argsOf("upsertblocklist"))).toContain(
      "example.com",
    );
    expect(JSON.stringify(tauri.argsOf("deleteblocklist"))).toContain("work");
  });

  it("settings_wrappers_read_and_replace_the_settings", async () => {
    tauri.on("getsettings", () => SETTINGS);
    tauri.on("updatesettings", () => undefined);
    const next = { default_preset: { ...PRESET, focus_min: 50 } };

    await expect(ipc.getSettings()).resolves.toEqual(SETTINGS);
    await ipc.updateSettings(next);

    expect(JSON.stringify(tauri.argsOf("updatesettings"))).toContain("50");
  });

  it("session_control_wrappers_invoke_one_command_each", async () => {
    const names = ["stopsession", "pausesession", "resumesession", "skipbreak"];
    for (const name of names) {
      tauri.on(name, () => undefined);
    }

    await ipc.stopSession();
    await ipc.pauseSession();
    await ipc.resumeSession();
    await ipc.skipBreak();

    for (const name of names) {
      expect(tauri.count(name)).toBe(1);
    }
  });

  it("stopSession_passes_the_typed_text_for_a_random_text_lock", async () => {
    tauri.on("stopsession", () => undefined);

    await ipc.stopSession("k3x9q");

    expect(JSON.stringify(tauri.argsOf("stopsession"))).toContain("k3x9q");
  });
});

describe("startSession", () => {
  it("startSession_sends_the_blocklist_preset_and_lock_and_no_time", async () => {
    tauri.on("startsession", () => undefined);

    await ipc.startSession({
      blocklist_id: "work",
      preset: PRESET,
      lock: { kind: "until_timer_ends" },
    });

    const sent = JSON.stringify(tauri.argsOf("startsession"));
    expect(sent).toContain("work");
    expect(sent).toContain("until_timer_ends");
    expect(sent).toContain("short_break_min");
    // No request carries a time (docs/architecture.md): the service owns the
    // clock, so the wrapper must not invent one.
    expect(sent).not.toMatch(
      /remaining|elapsed|started_at|ends_at|timestamp|unix_ms/i,
    );
  });
});

describe("errors", () => {
  it("a_service_error_reaches_the_caller_with_its_code", async () => {
    tauri.on("stopsession", () => {
      throw { code: "strict_session_locked", reason: "until_timer_ends" };
    });

    const failure = await ipc.stopSession().then(
      () => undefined,
      (e: unknown) => e,
    );

    expect((failure as { code?: string } | undefined)?.code).toBe(
      "strict_session_locked",
    );
  });

  it("a_not_found_error_keeps_a_different_code_from_a_locked_one", async () => {
    tauri.on("getblocklist", () => {
      throw { code: "not_found" };
    });

    const failure = await ipc.getBlocklist("nope").then(
      () => undefined,
      (e: unknown) => e,
    );

    expect((failure as { code?: string } | undefined)?.code).toBe("not_found");
  });
});

describe("onServiceEvent", () => {
  it("handlers_receive_the_flat_event_payload", async () => {
    const handler = vi.fn();
    await ipc.onServiceEvent(handler);

    tauri.emit({ type: "state_changed", state: FOCUS_STATE });
    tauri.emit({ type: "blocklist_changed", id: "work" });

    expect(handler).toHaveBeenCalledTimes(2);
    expect(handler).toHaveBeenNthCalledWith(1, {
      type: "state_changed",
      state: FOCUS_STATE,
    });
    expect(handler).toHaveBeenNthCalledWith(2, {
      type: "blocklist_changed",
      id: "work",
    });
  });

  it("the_returned_function_stops_listening", async () => {
    const stop = await ipc.onServiceEvent(vi.fn());
    expect(tauri.unlisten).not.toHaveBeenCalled();

    stop();

    expect(tauri.unlisten).toHaveBeenCalledTimes(1);
  });

  it("two_handlers_can_listen_at_once_and_each_is_called", async () => {
    const a = vi.fn();
    const b = vi.fn();
    await ipc.onServiceEvent(a);
    await ipc.onServiceEvent(b);

    tauri.emit({ type: "session_ended", outcome: "completed" });

    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
  });
});

describe("no network", () => {
  it("wrappers_never_call_fetch", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    tauri.on("getstate", () => IDLE_STATE);

    await ipc.getState();

    expect(fetchSpy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
