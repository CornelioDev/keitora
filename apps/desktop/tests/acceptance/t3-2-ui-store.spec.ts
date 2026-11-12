// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.2, the Pinia store for UI state (`@/stores/ui`).
// Derived from the task text and docs/ui-stack.md ("Pinia stores (UI state
// only)"):
//  - AC-12 the store keeps what the user picked on the Timer page
//      starts_with_nothing_selected_and_strict_off
//      selecting_a_blocklist_is_remembered_and_can_be_cleared
//      the_strict_request_can_be_switched
//  - AC-13 the store holds UI state only, never service data, and never
//    talks to the service
//      the_store_holds_no_session_or_blocklist_data
//      the_store_never_calls_the_service

import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { tauri } from "./support/t3-2-tauri-fake";

vi.mock("@tauri-apps/api/core", async () => {
  const { tauri } = await import("./support/t3-2-tauri-fake");
  return { invoke: tauri.invoke };
});
vi.mock("@tauri-apps/api/event", async () => {
  const { tauri } = await import("./support/t3-2-tauri-fake");
  return { listen: tauri.listen };
});

import { useUiStore } from "@/stores/ui";

beforeEach(() => {
  tauri.reset();
  setActivePinia(createPinia());
});

describe("useUiStore", () => {
  it("starts_with_nothing_selected_and_strict_off", () => {
    const store = useUiStore();

    expect(store.selectedBlocklistId).toBeNull();
    expect(store.strictRequested).toBe(false);
  });

  it("selecting_a_blocklist_is_remembered_and_can_be_cleared", () => {
    const store = useUiStore();

    store.selectBlocklist("work");
    expect(store.selectedBlocklistId).toBe("work");

    store.selectBlocklist(null);
    expect(store.selectedBlocklistId).toBeNull();
  });

  it("the_strict_request_can_be_switched", () => {
    const store = useUiStore();

    store.setStrictRequested(true);
    expect(store.strictRequested).toBe(true);

    store.setStrictRequested(false);
    expect(store.strictRequested).toBe(false);
  });

  it("the_store_holds_no_session_or_blocklist_data", () => {
    const store = useUiStore();

    const keys = Object.keys(store.$state).join(" ");
    expect(keys).not.toMatch(
      /phase|remaining|cycle|paused|lock\b|blocklists\b|settings|history/i,
    );
  });

  it("the_store_never_calls_the_service", () => {
    const store = useUiStore();

    store.selectBlocklist("work");
    store.setStrictRequested(true);

    expect(tauri.invoke).not.toHaveBeenCalled();
    expect(tauri.listen).not.toHaveBeenCalled();
  });
});
