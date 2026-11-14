// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.3, "theme following the OS". Criteria derived from
// the task line, docs/ui-stack.md ("Window": "Theme follows the OS,
// overridable in Settings" — the override is T-3.9) and T-0.9, which made the
// dark theme the `dark` class on the document's root element and put no token
// in a `prefers-color-scheme` media query, so the app itself has to follow
// the OS.
//
//  - AC-15 the app starts in the theme Windows asks for
//      the_app_starts_in_the_dark_theme_when_the_os_asks_for_it
//      the_app_starts_in_the_light_theme_when_the_os_asks_for_it
//  - AC-16 the theme follows the OS while the app runs, both ways
//      the_theme_follows_the_os_when_it_changes_to_dark
//      the_theme_follows_the_os_when_it_changes_back_to_light
//      the_theme_survives_moving_between_screens
//  - AC-17 the theme is the `dark` class on <html> and nothing else, which
//    is what the design tokens of T-0.9 are written against
//      the_theme_is_the_dark_class_on_the_root_element
//
// How the tests look: `prefers-color-scheme` is answered by a stand-in for
// `matchMedia` (jsdom has none) that the test changes like Windows would,
// notifying whoever listens.

import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";

import { IDLE_STATE, SETTINGS, tauri } from "./support/t3-2-tauri-fake";
import {
  NAV_ITEMS,
  cleanupShell,
  elementsOf,
  goTo,
  mountShell,
  setPrefersDark,
} from "./support/t3-3-shell";

vi.mock("@tauri-apps/api/core", async () => {
  const { tauri: fake } = await import("./support/t3-2-tauri-fake");
  return { invoke: fake.invoke };
});
vi.mock("@tauri-apps/api/event", async () => {
  const { tauri: fake } = await import("./support/t3-2-tauri-fake");
  return { listen: fake.listen };
});
vi.mock("@tauri-apps/api/window", async () => {
  const { appWindow } = await import("./support/t3-3-shell");
  return { getCurrentWindow: () => appWindow };
});
vi.mock("@tauri-apps/api/webviewWindow", async () => {
  const { appWindow } = await import("./support/t3-3-shell");
  return { getCurrentWebviewWindow: () => appWindow };
});

beforeEach(() => {
  tauri.reset();
  tauri.on("getstate", () => IDLE_STATE);
  tauri.on("getsettings", () => SETTINGS);
  tauri.on("listblocklists", () => ({ items: [], next_offset: null }));
});

afterEach(() => {
  cleanupShell();
  vi.clearAllMocks();
});

function isDark(): boolean {
  return document.documentElement.classList.contains("dark");
}

/** Tells the app the OS theme changed and lets it react. */
async function osSwitchesTo(theme: "dark" | "light"): Promise<void> {
  setPrefersDark(theme === "dark");
  await nextTick();
  await flushPromises();
}

describe("theme following the OS (T-3.3)", () => {
  it("the_app_starts_in_the_dark_theme_when_the_os_asks_for_it", async () => {
    await mountShell({ prefersDark: true });

    expect(isDark(), "Windows asks for the dark theme").toBe(true);
  });

  it("the_app_starts_in_the_light_theme_when_the_os_asks_for_it", async () => {
    await mountShell({ prefersDark: false });

    expect(isDark(), "Windows asks for the light theme").toBe(false);
  });

  it("the_theme_follows_the_os_when_it_changes_to_dark", async () => {
    await mountShell({ prefersDark: false });
    expect(isDark()).toBe(false);

    await osSwitchesTo("dark");

    expect(
      isDark(),
      "the window should turn dark when Windows does, without a restart",
    ).toBe(true);
  });

  it("the_theme_follows_the_os_when_it_changes_back_to_light", async () => {
    await mountShell({ prefersDark: true });
    expect(isDark()).toBe(true);

    await osSwitchesTo("light");

    expect(isDark()).toBe(false);
  });

  it("the_theme_survives_moving_between_screens", async () => {
    const shell = await mountShell({ prefersDark: true });

    for (const item of NAV_ITEMS) {
      await goTo(shell, item);
      expect(isDark(), `on the ${item.en} screen`).toBe(true);
    }
  });

  it("the_theme_is_the_dark_class_on_the_root_element", async () => {
    const shell = await mountShell({ prefersDark: true });

    // The tokens of T-0.9 live on <html>, under `.dark` for the dark theme,
    // so nothing below it carries the theme.
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.body.classList.contains("dark")).toBe(false);
    const inside = elementsOf(shell).filter((element) =>
      element.classList.contains("dark"),
    );
    expect(
      inside.map((element) => element.tagName),
      "the theme belongs on <html>, not on an element of the shell",
    ).toEqual([]);

    await osSwitchesTo("light");

    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
