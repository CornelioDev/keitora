// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.3, the keyboard and screen-reader side of the app
// shell. Criteria derived from AGENTS.md ("Accessibility | WCAG 2.2 AA:
// keyboard reachable, visible focus, labels on all controls") and
// docs/ui-stack.md ("Custom title bar ... with accessible window controls").
// The full walkthrough of every screen is T-3.14; these tests cover the shell
// the other screens are built in.
//
//  - AC-21 everything the shell offers is reachable with the keyboard: every
//    control is a link or a button, focusable, and none is hidden from the
//    keyboard or from a screen reader
//      every_control_of_the_shell_can_take_focus
//      no_control_is_taken_out_of_the_tab_order
//      no_control_is_hidden_from_a_screen_reader
//  - AC-22 focus is visible on every control (WCAG 2.2, 2.4.11/2.4.13)
//      every_control_shows_a_visible_focus_ring
//  - AC-23 every control has a name a screen reader announces (4.1.2)
//      every_control_of_the_shell_has_an_accessible_name
//  - AC-24 the window is laid out in landmarks, and the reading order is the
//    order of the window: title bar, sidebar, screen (1.3.1, 2.4.3)
//      the_shell_has_a_navigation_landmark_and_a_main_landmark
//      the_screen_is_rendered_inside_the_main_landmark
//      the_tab_order_follows_the_window_from_top_left_to_the_screen
//
// The language of the document (<html lang>, WCAG 2.2, 3.1.1) is set by
// src/main.ts and covered by t0-10-i18n-app.spec.ts, so no test repeats it
// here.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { IDLE_STATE, SETTINGS, tauri } from "./support/t3-2-tauri-fake";
import {
  NAV_ITEMS,
  WINDOW_CONTROLS,
  accessibleName,
  cleanupShell,
  describeElement,
  elementsOf,
  findIn,
  goTo,
  interactiveElements,
  mainArea,
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

/** Whether an element or one of its parents is hidden from assistive tech. */
function hiddenFromScreenReader(element: HTMLElement): boolean {
  let current: HTMLElement | null = element;
  while (current !== null) {
    if (current.getAttribute("aria-hidden") === "true") return true;
    if (current.hasAttribute("hidden")) return true;
    current = current.parentElement;
  }
  return false;
}

/** Whether a focus ring is styled for this control (Tailwind's variant). */
function hasFocusStyle(element: HTMLElement): boolean {
  return /\bfocus-visible:|\bfocus:/.test(element.className);
}

function inDocumentOrder(first: Element, second: Element): boolean {
  return Boolean(
    first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING,
  );
}

/** The controls of the shell, in the order a screen reader reads them. */
function controls(shell: Shell): HTMLElement[] {
  return interactiveElements(shell);
}

describe("the shell's accessibility (T-3.3)", () => {
  it("every_control_of_the_shell_can_take_focus", async () => {
    const shell = await mountShell();

    for (const element of controls(shell)) {
      element.focus();
      expect(
        document.activeElement,
        `${describeElement(element)} should take keyboard focus`,
      ).toBe(element);
    }
  });

  it("no_control_is_taken_out_of_the_tab_order", async () => {
    const shell = await mountShell();

    for (const element of controls(shell)) {
      const tabindex = element.getAttribute("tabindex");
      expect(
        tabindex === null || Number(tabindex) >= 0,
        `${describeElement(element)} should stay in the tab order`,
      ).toBe(true);
      expect(
        element.hasAttribute("disabled"),
        `${describeElement(element)} should not be disabled in the shell`,
      ).toBe(false);
    }
  });

  it("no_control_is_hidden_from_a_screen_reader", async () => {
    const shell = await mountShell();

    for (const element of controls(shell)) {
      expect(
        hiddenFromScreenReader(element),
        `${describeElement(element)} should be announced, not hidden`,
      ).toBe(false);
    }
  });

  it("every_control_shows_a_visible_focus_ring", async () => {
    const shell = await mountShell();

    for (const element of controls(shell)) {
      expect(
        hasFocusStyle(element),
        `${describeElement(element)} should style its focus (WCAG 2.2, 2.4.11)`,
      ).toBe(true);
    }
  });

  it("every_control_of_the_shell_has_an_accessible_name", async () => {
    const shell = await mountShell();

    for (const element of controls(shell)) {
      expect(
        accessibleName(element),
        `${describeElement(element)} should have a name a screen reader announces`,
      ).not.toBe("");
    }
  });

  it("the_shell_has_a_navigation_landmark_and_a_main_landmark", async () => {
    const shell = await mountShell();

    const navigations = elementsOf(shell).filter(
      (element) =>
        element.tagName === "NAV" ||
        element.getAttribute("role") === "navigation",
    );
    expect(navigations.length, "one navigation landmark (the sidebar)").toBe(1);
    expect(
      elementsOf(shell).filter((element) => element.tagName === "MAIN").length,
      "one main landmark (the screen)",
    ).toBe(1);
  });

  it("the_screen_is_rendered_inside_the_main_landmark", async () => {
    const shell = await mountShell();

    for (const item of NAV_ITEMS) {
      await goTo(shell, item);
      const main = mainArea(shell);
      expect(
        main.contains(navLink(shell, item)),
        "the sidebar belongs outside the main landmark",
      ).toBe(false);
      expect(
        (main.textContent ?? "").trim(),
        `the ${item.en} screen should render inside <main>`,
      ).not.toBe("");
    }
  });

  it("the_tab_order_follows_the_window_from_top_left_to_the_screen", async () => {
    const shell = await mountShell();
    const nav = findIn(shell, "nav");
    expect(nav).not.toBeNull();

    for (const control of WINDOW_CONTROLS) {
      expect(
        inDocumentOrder(windowControl(shell, control), nav as Element),
        `the ${control} control is read before the sidebar`,
      ).toBe(true);
    }
    expect(
      inDocumentOrder(nav as Element, mainArea(shell)),
      "the sidebar is read before the screen",
    ).toBe(true);
    for (const [index, item] of NAV_ITEMS.entries()) {
      if (index === 0) continue;
      expect(
        inDocumentOrder(
          navLink(shell, NAV_ITEMS[index - 1]),
          navLink(shell, item),
        ),
        `${item.en} is read after ${NAV_ITEMS[index - 1].en}`,
      ).toBe(true);
    }
  });
});
