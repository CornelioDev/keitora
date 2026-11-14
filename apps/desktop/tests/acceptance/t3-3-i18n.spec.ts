// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.3, the text of the app shell. Criteria derived
// from AGENTS.md ("No hard-coded UI strings. All user-facing text goes
// through `vue-i18n` (`en` and `es` from day one)"), ADR-0007 and the MVP
// mockups, which are in Spanish and give the labels of the four screens.
//
//  - AC-18 every name the shell shows comes from the locale files: the four
//    navigation items, the headings of the screens and the window controls
//    read in English and in Spanish
//      the_navigation_items_are_named_in_english
//      the_navigation_items_are_named_in_spanish
//      every_screen_shows_its_name_as_the_heading_in_spanish
//      the_window_controls_are_named_in_both_languages
//  - AC-19 the text follows the language while the app runs, so nothing in
//    the shell is hard-coded (the language setting of T-3.9 changes it)
//      changing_the_language_renames_the_navigation_items
//      changing_the_language_renames_the_window_controls
//  - AC-20 the shell uses no message that the locale files lack: vue-i18n
//    warns about a missing key, and nothing it shows is a key path
//      the_shell_shows_no_missing_translation

import { flushPromises } from "@vue/test-utils";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";
import { nextTick } from "vue";

import { IDLE_STATE, SETTINGS, tauri } from "./support/t3-2-tauri-fake";
import {
  NAV_ITEMS,
  WINDOW_CONTROLS,
  accessibleName,
  cleanupShell,
  goTo,
  mainHeading,
  mountShell,
  navLink,
  windowControl,
  type Shell,
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

let warn: MockInstance<typeof console.warn>;
let error: MockInstance<typeof console.error>;

beforeEach(() => {
  tauri.reset();
  tauri.on("getstate", () => IDLE_STATE);
  tauri.on("getsettings", () => SETTINGS);
  tauri.on("listblocklists", () => ({ items: [], next_offset: null }));
  warn = vi.spyOn(console, "warn");
  error = vi.spyOn(console, "error");
});

afterEach(() => {
  cleanupShell();
  warn.mockRestore();
  error.mockRestore();
  vi.clearAllMocks();
});

function navNames(shell: Shell): string[] {
  return NAV_ITEMS.map((item) => accessibleName(navLink(shell, item)));
}

function controlNames(shell: Shell): string[] {
  return WINDOW_CONTROLS.map((control) =>
    accessibleName(windowControl(shell, control)),
  );
}

/** Switches the app's language, as the setting of T-3.9 will. */
async function switchTo(shell: Shell, locale: "en" | "es"): Promise<void> {
  shell.i18n.global.locale.value = locale;
  await nextTick();
  await flushPromises();
}

describe("the shell's text (T-3.3)", () => {
  it("the_navigation_items_are_named_in_english", async () => {
    const shell = await mountShell({ locale: "en" });

    expect(navNames(shell)).toEqual(NAV_ITEMS.map((item) => item.en));
  });

  it("the_navigation_items_are_named_in_spanish", async () => {
    const shell = await mountShell({ locale: "es" });

    expect(
      navNames(shell),
      "the Spanish labels of the MVP mockups (docs/assets/mockups)",
    ).toEqual(NAV_ITEMS.map((item) => item.es));
  });

  it("every_screen_shows_its_name_as_the_heading_in_spanish", async () => {
    const shell = await mountShell({ locale: "es" });

    for (const item of NAV_ITEMS) {
      await goTo(shell, item);
      expect(mainHeading(shell), `the ${item.es} screen`).toBe(item.es);
    }
  });

  it("the_window_controls_are_named_in_both_languages", async () => {
    const english = await mountShell({ locale: "en" });
    const inEnglish = controlNames(english);
    cleanupShell();
    const spanish = await mountShell({ locale: "es" });

    const inSpanish = controlNames(spanish);

    for (const [index, control] of WINDOW_CONTROLS.entries()) {
      expect(inEnglish[index], `the ${control} control in English`).not.toBe(
        "",
      );
      expect(inSpanish[index], `the ${control} control in Spanish`).not.toBe(
        "",
      );
      expect(
        inSpanish[index],
        `the ${control} control should be translated, not the same text in both languages`,
      ).not.toBe(inEnglish[index]);
    }
  });

  it("changing_the_language_renames_the_navigation_items", async () => {
    const shell = await mountShell({ locale: "en" });
    expect(navNames(shell)).toEqual(NAV_ITEMS.map((item) => item.en));

    await switchTo(shell, "es");

    expect(
      navNames(shell),
      "the labels come from vue-i18n, so they follow the language without a restart",
    ).toEqual(NAV_ITEMS.map((item) => item.es));

    await switchTo(shell, "en");

    expect(navNames(shell)).toEqual(NAV_ITEMS.map((item) => item.en));
  });

  it("changing_the_language_renames_the_window_controls", async () => {
    const shell = await mountShell({ locale: "en" });
    const before = controlNames(shell);

    await switchTo(shell, "es");

    expect(controlNames(shell)).not.toEqual(before);
  });

  it("the_shell_shows_no_missing_translation", async () => {
    for (const locale of ["en", "es"] as const) {
      const shell = await mountShell({ locale });
      for (const item of NAV_ITEMS) {
        await goTo(shell, item);
        const text = (shell.host.textContent ?? "").replace(/\s+/g, " ").trim();
        expect(
          text,
          `${locale}: the ${item.en} screen should show its text`,
        ).not.toBe("");
      }
      cleanupShell();
    }

    const logged = [...warn.mock.calls, ...error.mock.calls].map((args) =>
      args.map(String).join(" "),
    );
    expect(
      logged.filter((message) => /i18n|translat|missing/i.test(message)),
      "vue-i18n warns when a message is missing",
    ).toEqual([]);
  });
});
