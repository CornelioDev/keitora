// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.2, the vue-query composables and event-driven
// invalidation. Derived from the task text (no acceptance line in the roadmap):
//  - AC-6 useEngineState serves the service's state through vue-query
//      engine_state_loads_from_the_service
//      engine_state_reports_an_error_when_the_service_fails
//  - AC-7 useBlocklists serves every blocklist, following next_offset
//      blocklists_load_every_page
//  - AC-8 useSettings serves the settings and can replace them
//      settings_load_from_the_service
//      settings_update_sends_the_new_settings_and_refreshes
//      settings_update_rejected_by_the_service_surfaces_the_error
//  - AC-9 blocklist mutations go through the service and refresh the list
//      blocklist_mutations_refresh_the_list
//  - AC-10 queries are invalidated on service events
//      state_changed_event_updates_the_engine_state
//      session_ended_event_refetches_the_engine_state
//      blocklist_changed_event_refetches_blocklists_only
//  - AC-11 listeners are released with the component
//      engine_state_listens_to_events_and_stops_on_unmount

import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  FOCUS_STATE,
  IDLE_STATE,
  PRESET,
  SETTINGS,
  SOCIAL_LIST,
  WORK_LIST,
  eventually,
  tauri,
} from "./support/t3-2-tauri-fake";
import { withSetup } from "./support/t3-2-mount";

vi.mock("@tauri-apps/api/core", async () => {
  const { tauri } = await import("./support/t3-2-tauri-fake");
  return { invoke: tauri.invoke };
});
vi.mock("@tauri-apps/api/event", async () => {
  const { tauri } = await import("./support/t3-2-tauri-fake");
  return { listen: tauri.listen };
});

import { useBlocklists } from "@/composables/useBlocklists";
import { useEngineState } from "@/composables/useEngineState";
import { useSettings } from "@/composables/useSettings";

beforeEach(() => {
  tauri.reset();
});

describe("useEngineState", () => {
  it("engine_state_loads_from_the_service", async () => {
    tauri.on("getstate", () => FOCUS_STATE);

    const { result } = withSetup(() => useEngineState());

    await eventually(() => {
      expect(result.data.value).toEqual(FOCUS_STATE);
    });
  });

  it("engine_state_reports_an_error_when_the_service_fails", async () => {
    tauri.on("getstate", () => {
      throw { code: "internal" };
    });

    const { result } = withSetup(() => useEngineState());

    await eventually(() => {
      expect(result.error.value).toBeTruthy();
    });
    expect(result.data.value).toBeUndefined();
  });

  it("state_changed_event_updates_the_engine_state", async () => {
    let current: unknown = IDLE_STATE;
    tauri.on("getstate", () => current);
    const { result } = withSetup(() => useEngineState());
    await eventually(() => {
      expect(result.data.value).toEqual(IDLE_STATE);
    });
    await eventually(() => {
      expect(tauri.listenerCount).toBeGreaterThan(0);
    });

    current = FOCUS_STATE;
    tauri.emit({ type: "state_changed", state: FOCUS_STATE });

    await eventually(() => {
      expect(result.data.value).toEqual(FOCUS_STATE);
    });
  });

  it("session_ended_event_refetches_the_engine_state", async () => {
    let current: unknown = FOCUS_STATE;
    tauri.on("getstate", () => current);
    const { result } = withSetup(() => useEngineState());
    await eventually(() => {
      expect(result.data.value).toEqual(FOCUS_STATE);
    });
    await eventually(() => {
      expect(tauri.listenerCount).toBeGreaterThan(0);
    });

    current = IDLE_STATE;
    tauri.emit({ type: "session_ended", outcome: "completed" });

    await eventually(() => {
      expect(result.data.value).toEqual(IDLE_STATE);
    });
  });

  it("engine_state_listens_to_events_and_stops_on_unmount", async () => {
    tauri.on("getstate", () => IDLE_STATE);
    const { wrapper, result } = withSetup(() => useEngineState());
    await eventually(() => {
      expect(result.data.value).toEqual(IDLE_STATE);
    });
    await eventually(() => {
      expect(tauri.listenerCount).toBeGreaterThan(0);
    });
    expect(tauri.unlisten).not.toHaveBeenCalled();

    wrapper.unmount();
    await flushPromises();

    expect(tauri.unlisten).toHaveBeenCalled();
  });
});

describe("useBlocklists", () => {
  it("blocklists_load_every_page", async () => {
    tauri.on("listblocklists", (args) => {
      const sent = JSON.stringify(args);
      return sent.includes('"offset":0')
        ? { items: [WORK_LIST], next_offset: 1 }
        : { items: [SOCIAL_LIST], next_offset: null };
    });

    const { result } = withSetup(() => useBlocklists());

    await eventually(() => {
      expect(result.data.value).toEqual([WORK_LIST, SOCIAL_LIST]);
    });
    // Pages are capped at 100 items by the protocol.
    for (const args of tauri.argsOf("listblocklists")) {
      const match = /"limit":(\d+)/.exec(JSON.stringify(args));
      expect(Number(match?.[1])).toBeLessThanOrEqual(100);
    }
  });

  it("blocklist_changed_event_refetches_blocklists_only", async () => {
    tauri.on("listblocklists", () => ({
      items: [WORK_LIST],
      next_offset: null,
    }));
    tauri.on("getsettings", () => SETTINGS);
    const { result } = withSetup(() => {
      return { lists: useBlocklists(), settings: useSettings() };
    });
    await eventually(() => {
      expect(result.lists.data.value).toEqual([WORK_LIST]);
      expect(result.settings.data.value).toEqual(SETTINGS);
    });
    await eventually(() => {
      expect(tauri.listenerCount).toBeGreaterThan(0);
    });
    const listsBefore = tauri.count("listblocklists");
    const settingsBefore = tauri.count("getsettings");

    tauri.on("listblocklists", () => ({
      items: [WORK_LIST, SOCIAL_LIST],
      next_offset: null,
    }));
    tauri.emit({ type: "blocklist_changed", id: "social" });

    await eventually(() => {
      expect(result.lists.data.value).toEqual([WORK_LIST, SOCIAL_LIST]);
    });
    expect(tauri.count("listblocklists")).toBeGreaterThan(listsBefore);
    expect(tauri.count("getsettings")).toBe(settingsBefore);
  });

  it("blocklist_mutations_refresh_the_list", async () => {
    let items = [WORK_LIST];
    tauri.on("listblocklists", () => ({ items, next_offset: null }));
    tauri.on("upsertblocklist", () => {
      items = [WORK_LIST, SOCIAL_LIST];
    });
    tauri.on("deleteblocklist", () => {
      items = [WORK_LIST];
    });
    const { result } = withSetup(() => useBlocklists());
    await eventually(() => {
      expect(result.data.value).toEqual([WORK_LIST]);
    });

    await result.upsertBlocklist({
      ...SOCIAL_LIST,
      targets: [],
      exceptions: [],
      block_during_breaks: false,
    });
    await eventually(() => {
      expect(result.data.value).toEqual([WORK_LIST, SOCIAL_LIST]);
    });

    await result.deleteBlocklist("social");
    await eventually(() => {
      expect(result.data.value).toEqual([WORK_LIST]);
    });
  });
});

describe("useSettings", () => {
  it("settings_load_from_the_service", async () => {
    tauri.on("getsettings", () => SETTINGS);

    const { result } = withSetup(() => useSettings());

    await eventually(() => {
      expect(result.data.value).toEqual(SETTINGS);
    });
  });

  it("settings_update_sends_the_new_settings_and_refreshes", async () => {
    let stored = SETTINGS;
    tauri.on("getsettings", () => stored);
    tauri.on("updatesettings", (args) => {
      void args;
      stored = { default_preset: { ...PRESET, focus_min: 50 } };
    });
    const { result } = withSetup(() => useSettings());
    await eventually(() => {
      expect(result.data.value).toEqual(SETTINGS);
    });

    await result.updateSettings({
      default_preset: { ...PRESET, focus_min: 50 },
    });

    expect(JSON.stringify(tauri.argsOf("updatesettings"))).toContain("50");
    await eventually(() => {
      expect(result.data.value?.default_preset.focus_min).toBe(50);
    });
  });

  it("settings_update_rejected_by_the_service_surfaces_the_error", async () => {
    tauri.on("getsettings", () => SETTINGS);
    tauri.on("updatesettings", () => {
      throw { code: "validation", field: "settings", kind: "out_of_range" };
    });
    const { result } = withSetup(() => useSettings());
    await eventually(() => {
      expect(result.data.value).toEqual(SETTINGS);
    });

    const failure = await result
      .updateSettings({ default_preset: { ...PRESET, focus_min: 9999 } })
      .then(
        () => undefined,
        (e: unknown) => e,
      );

    expect((failure as { code?: string } | undefined)?.code).toBe("validation");
    expect(result.data.value).toEqual(SETTINGS);
  });
});
