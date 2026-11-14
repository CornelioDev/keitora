// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.3, the custom title bar of the app shell.
// Criteria derived from the task line, docs/ui-stack.md ("Window": "Custom
// title bar (`decorations: false`) with accessible window controls"), the MVP
// mockups (the bar spans the top of the window, with minimize, maximize and
// close on the right) and the maintainer's decision for this task: the window
// controls need the first Tauri permissions of the app, and exactly four are
// approved — minimize, toggle-maximize, close and start-dragging.
//
//  - AC-10 the window has no native decorations: the app draws the title bar
//      the_window_has_no_native_decorations
//  - AC-11 the title bar sits at the top of the window and can be dragged
//      the_title_bar_is_the_first_thing_in_the_window
//      the_title_bar_has_a_drag_region
//      the_window_controls_are_not_part_of_the_drag_region
//  - AC-12 it has three controls — minimize, maximize/restore and close —
//    each a button with a name a screen reader announces
//      the_title_bar_has_the_three_window_controls
//      every_window_control_has_an_accessible_name
//  - AC-13 each control acts on the window it runs in, and only through the
//    approved Tauri methods; the shell touches the window on its own
//    initiative (failure path: a method nobody approved, such as `theme()`
//    for the theme, would need a fifth permission)
//      clicking_minimize_minimizes_the_window
//      clicking_maximize_toggles_the_maximized_state
//      clicking_close_closes_the_window
//      starting_the_shell_does_not_act_on_the_window
//  - AC-14 the app's capabilities grant exactly the four approved
//    permissions, so a fifth one fails the suite
//      the_app_grants_exactly_the_four_approved_window_permissions

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { IDLE_STATE, SETTINGS, tauri } from "./support/t3-2-tauri-fake";
import {
  WINDOW_CONTROLS,
  accessibleName,
  appWindow,
  cleanupShell,
  describeElement,
  elementsOf,
  findIn,
  mountShell,
  windowControl,
  type Shell,
  type WindowControl,
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
  const { appWindow: fake } = await import("./support/t3-3-shell");
  return { getCurrentWindow: () => fake };
});
vi.mock("@tauri-apps/api/webviewWindow", async () => {
  const { appWindow: fake } = await import("./support/t3-3-shell");
  return { getCurrentWebviewWindow: () => fake };
});

// In the jsdom environment the global URL is jsdom's, which does not resolve
// file: URLs, so the path is built with Node.js' path functions.
const tauriRoot = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "src-tauri",
);

/** The permissions the maintainer approved for T-3.3, and no others. */
const APPROVED_PERMISSIONS = [
  "core:window:allow-close",
  "core:window:allow-minimize",
  "core:window:allow-start-dragging",
  "core:window:allow-toggle-maximize",
];

// Window methods that would each need a permission nobody approved: the
// capability grants minimize, toggle-maximize, close and start-dragging only,
// so the theme comes from `prefers-color-scheme` and nothing reads or listens
// to the window's state.
const FORBIDDEN_METHODS = [
  "maximize",
  "unmaximize",
  "destroy",
  "hide",
  "show",
  "setDecorations",
  "theme",
  "isMaximized",
  "onResized",
  "listen",
] as const;

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

function dragRegion(shell: Shell): HTMLElement {
  const region = findIn(shell, "[data-tauri-drag-region]");
  expect(
    region,
    "the title bar should be draggable: an element with data-tauri-drag-region",
  ).not.toBeNull();
  return region as HTMLElement;
}

function expectUntouched(methods: readonly string[]): void {
  for (const method of methods) {
    const spy = appWindow[method as keyof typeof appWindow];
    if (!vi.isMockFunction(spy)) continue;
    expect(
      spy.mock.calls.length,
      `the shell should not call window.${method}(): it is not one of the four approved permissions`,
    ).toBe(0);
  }
}

async function clickControl(
  shell: Shell,
  control: WindowControl,
): Promise<void> {
  windowControl(shell, control).click();
  await flushPromises();
}

/** Every permission granted by the app's capability files. */
function grantedPermissions(): string[] {
  const directory = join(tauriRoot, "capabilities");
  expect(existsSync(directory), "src-tauri/capabilities should exist").toBe(
    true,
  );
  const granted: string[] = [];
  for (const entry of readdirSync(directory, {
    withFileTypes: true,
    recursive: true,
  })) {
    if (!entry.isFile() || !/\.(json|toml)$/.test(entry.name)) continue;
    expect(
      entry.name.endsWith(".json"),
      `${entry.name}: the capabilities are JSON, as in T-0.6`,
    ).toBe(true);
    const file = join(entry.parentPath, entry.name);
    const parsed = JSON.parse(readFileSync(file, "utf8")) as {
      permissions?: (string | { identifier?: string })[];
    };
    for (const permission of parsed.permissions ?? []) {
      granted.push(
        typeof permission === "string"
          ? permission
          : (permission.identifier ?? JSON.stringify(permission)),
      );
    }
  }
  return granted;
}

describe("custom title bar (T-3.3)", () => {
  it("the_window_has_no_native_decorations", () => {
    const config = JSON.parse(
      readFileSync(join(tauriRoot, "tauri.conf.json"), "utf8"),
    ) as { app?: { windows?: { label?: string; decorations?: boolean }[] } };

    const windows = config.app?.windows ?? [];
    expect(
      windows.length,
      "tauri.conf.json should define the main window",
    ).toBe(1);
    expect(
      windows[0]?.decorations,
      "the app draws its own title bar, so the window has decorations: false",
    ).toBe(false);
  });

  it("the_app_grants_exactly_the_four_approved_window_permissions", () => {
    const granted = grantedPermissions();

    expect(
      [...granted].sort(),
      "the maintainer approved exactly these permissions for the title bar; adding another one needs a new decision",
    ).toEqual(APPROVED_PERMISSIONS);
  });

  it("the_title_bar_is_the_first_thing_in_the_window", async () => {
    const shell = await mountShell();
    const region = dragRegion(shell);
    const nav = findIn(shell, "nav");
    const main = findIn(shell, "main");

    for (const [name, element] of [
      ["the sidebar", nav],
      ["the screen", main],
    ] as const) {
      expect(element, `the shell should render ${name}`).not.toBeNull();
      expect(
        Boolean(
          region.compareDocumentPosition(element as Element) &
          Node.DOCUMENT_POSITION_FOLLOWING,
        ),
        `the title bar should come before ${name}, as it does in the window`,
      ).toBe(true);
    }
  });

  it("the_title_bar_has_a_drag_region", async () => {
    const shell = await mountShell();

    const region = dragRegion(shell);
    const nav = findIn(shell, "nav");

    expect(nav, "the shell should render the sidebar").not.toBeNull();
    for (const control of WINDOW_CONTROLS) {
      const button = windowControl(shell, control);
      expect(
        Boolean(
          button.compareDocumentPosition(nav as Element) &
          Node.DOCUMENT_POSITION_FOLLOWING,
        ),
        `the ${control} control should be in the title bar, above the sidebar`,
      ).toBe(true);
    }
    expect(
      region.isConnected,
      "the drag region should be part of the window",
    ).toBe(true);
  });

  it("the_window_controls_are_not_part_of_the_drag_region", async () => {
    const shell = await mountShell();

    for (const control of WINDOW_CONTROLS) {
      const button = windowControl(shell, control);
      expect(
        button.hasAttribute("data-tauri-drag-region"),
        `${describeElement(button)} would be dragged instead of clicked`,
      ).toBe(false);
      expect(
        [...button.querySelectorAll("[data-tauri-drag-region]")].length,
        `nothing inside the ${control} control should be a drag region`,
      ).toBe(0);
    }
  });

  it("the_title_bar_has_the_three_window_controls", async () => {
    const shell = await mountShell();

    for (const control of WINDOW_CONTROLS) {
      const button = windowControl(shell, control);
      expect(
        button.tagName,
        `the ${control} control should be a <button>`,
      ).toBe("BUTTON");
      expect(
        button.getAttribute("type"),
        `the ${control} control should not submit anything`,
      ).toBe("button");
    }
    const marked = elementsOf(shell).filter((element) =>
      element.hasAttribute("data-window-control"),
    );
    expect(marked.length, "the title bar has three controls, no more").toBe(3);
  });

  it("every_window_control_has_an_accessible_name", async () => {
    const shell = await mountShell();

    const names = WINDOW_CONTROLS.map((control) =>
      accessibleName(windowControl(shell, control)),
    );

    for (const [index, control] of WINDOW_CONTROLS.entries()) {
      expect(
        names[index],
        `the ${control} control should have a name a screen reader announces`,
      ).not.toBe("");
    }
    expect(new Set(names).size, "each control has its own name").toBe(3);
  });

  it("clicking_minimize_minimizes_the_window", async () => {
    const shell = await mountShell();

    await clickControl(shell, "minimize");

    expect(appWindow.minimize).toHaveBeenCalledTimes(1);
    expect(appWindow.close).not.toHaveBeenCalled();
    expect(appWindow.toggleMaximize).not.toHaveBeenCalled();
    expectUntouched(FORBIDDEN_METHODS);
  });

  it("clicking_maximize_toggles_the_maximized_state", async () => {
    const shell = await mountShell();

    await clickControl(shell, "toggle-maximize");

    expect(appWindow.toggleMaximize).toHaveBeenCalledTimes(1);
    expect(appWindow.minimize).not.toHaveBeenCalled();
    expect(appWindow.close).not.toHaveBeenCalled();
    expectUntouched(FORBIDDEN_METHODS);
  });

  it("clicking_close_closes_the_window", async () => {
    const shell = await mountShell();

    await clickControl(shell, "close");

    expect(appWindow.close).toHaveBeenCalledTimes(1);
    expect(appWindow.minimize).not.toHaveBeenCalled();
    expect(appWindow.toggleMaximize).not.toHaveBeenCalled();
    expectUntouched(FORBIDDEN_METHODS);
  });

  it("starting_the_shell_does_not_act_on_the_window", async () => {
    await mountShell();

    expectUntouched([
      "minimize",
      "toggleMaximize",
      "close",
      ...FORBIDDEN_METHODS,
    ]);
  });
});
