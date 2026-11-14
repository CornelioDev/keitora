// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.3, the sidebar of the app shell: "App shell:
// shadcn-vue `Sidebar` (Timer, Blocklists, History, Settings)". Criteria
// derived from the task line, docs/ui-stack.md ("Window": "Fixed 220 px
// sidebar: Timer, Blocklists, History, Settings"; "Don't build UI from
// scratch", AGENTS.md) and the MVP mockups (docs/assets/mockups), where the
// sidebar shows the product name at the top and marks the open screen.
//
//  - AC-6 the sidebar is built on the shadcn-vue `sidebar` component that
//    T-0.8 installed (src/components/ui/sidebar), not on a new one
//      the_sidebar_is_the_shadcn_vue_sidebar_component
//  - AC-7 it lists the four screens, in the order of the roadmap, as links
//    inside a navigation landmark, each with the screen's name
//      the_sidebar_lists_the_four_screens_in_order
//      every_navigation_item_is_a_link_inside_a_navigation_landmark
//  - AC-8 clicking an item opens its screen, and only the open screen is
//    marked as current
//      clicking_an_item_opens_its_screen
//      only_the_open_screen_is_marked_as_current
//      the_current_item_follows_a_navigation_started_elsewhere
//  - AC-9 the sidebar is fixed at 220 px wide and shows the product name
//      the_sidebar_is_220_px_wide
//      the_sidebar_shows_the_product_name
//      the_sidebar_is_always_visible

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { IDLE_STATE, SETTINGS, tauri } from "./support/t3-2-tauri-fake";
import {
  NAV_ITEMS,
  accessibleName,
  cleanupShell,
  elementsOf,
  goTo,
  mountShell,
  navLink,
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

// In the jsdom environment the global URL is jsdom's, which does not resolve
// file: URLs, so the path is built with Node.js' path functions.
const appRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

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

/** Whether a link is marked as the screen the user is on (WCAG 2.2, 4.1.2). */
function isCurrent(link: HTMLAnchorElement): boolean {
  return link.getAttribute("aria-current") === "page";
}

/** The text of the sidebar, which is everything outside the main area. */
function sidebarText(shell: Shell): string {
  const clone = shell.host.cloneNode(true) as HTMLElement;
  clone.querySelector("main")?.remove();
  return (clone.textContent ?? "").replace(/\s+/g, " ").trim();
}

/** Whether the sidebar is 220 px wide, set inline or in the app's styles. */
function sidebarWidth(shell: Shell): string | undefined {
  for (const element of elementsOf(shell)) {
    const value = element.style.getPropertyValue("--sidebar-width").trim();
    if (value !== "") return value;
  }
  const stylesDir = join(appRoot, "src", "styles");
  for (const entry of readdirSync(stylesDir, {
    withFileTypes: true,
    recursive: true,
  })) {
    if (!entry.isFile() || !entry.name.endsWith(".css")) continue;
    const css = readFileSync(join(entry.parentPath, entry.name), "utf8");
    const match = /--sidebar-width\s*:\s*([^;}]+)/.exec(css);
    if (match !== null) return (match[1] ?? "").trim();
  }
  return undefined;
}

describe("app sidebar (T-3.3)", () => {
  it("the_sidebar_is_the_shadcn_vue_sidebar_component", async () => {
    const shell = await mountShell();
    const { Sidebar, SidebarProvider } =
      await import("@/components/ui/sidebar");

    expect(
      shell.wrapper.findComponent(SidebarProvider).exists(),
      "the shell should use the shadcn-vue sidebar installed in T-0.8",
    ).toBe(true);
    expect(shell.wrapper.findComponent(Sidebar).exists()).toBe(true);
  });

  it("the_sidebar_lists_the_four_screens_in_order", async () => {
    const shell = await mountShell();

    const links = NAV_ITEMS.map((item) => navLink(shell, item));
    for (const [index, item] of NAV_ITEMS.entries()) {
      expect(
        accessibleName(links[index]),
        `the item that opens ${item.path}`,
      ).toBe(item.en);
    }
    for (let index = 1; index < links.length; index += 1) {
      const order = links[index - 1].compareDocumentPosition(links[index]);
      expect(
        Boolean(order & Node.DOCUMENT_POSITION_FOLLOWING),
        `${NAV_ITEMS[index].en} should come after ${NAV_ITEMS[index - 1].en}`,
      ).toBe(true);
    }
  });

  it("every_navigation_item_is_a_link_inside_a_navigation_landmark", async () => {
    const shell = await mountShell();

    for (const item of NAV_ITEMS) {
      const link = navLink(shell, item);
      expect(
        link.closest("nav") ?? link.closest('[role="navigation"]'),
        `${item.en} should sit in a navigation landmark (<nav>)`,
      ).not.toBeNull();
      expect(
        link.hasAttribute("href"),
        `${item.en} should be a real link`,
      ).toBe(true);
    }
  });

  it("clicking_an_item_opens_its_screen", async () => {
    const shell = await mountShell();

    for (const item of NAV_ITEMS) {
      navLink(shell, item).click();
      await flushPromises();

      expect(
        shell.router.currentRoute.value.name,
        `clicking ${item.en} should open ${item.path}`,
      ).toBe(item.name);
    }
  });

  it("only_the_open_screen_is_marked_as_current", async () => {
    const shell = await mountShell();

    for (const item of NAV_ITEMS) {
      await goTo(shell, item);

      const current = NAV_ITEMS.filter((other) =>
        isCurrent(navLink(shell, other)),
      ).map((other) => other.name);
      expect(
        current,
        `only ${item.en} should be marked aria-current="page"`,
      ).toEqual([item.name]);
    }
  });

  it("the_current_item_follows_a_navigation_started_elsewhere", async () => {
    const shell = await mountShell();
    const settings = NAV_ITEMS[3];

    await shell.router.push(settings.path);
    await flushPromises();

    expect(isCurrent(navLink(shell, settings))).toBe(true);
    expect(isCurrent(navLink(shell, NAV_ITEMS[0]))).toBe(false);
  });

  it("the_sidebar_is_220_px_wide", async () => {
    const shell = await mountShell();

    const width = sidebarWidth(shell);

    expect(
      width,
      "the sidebar's width should be set through --sidebar-width (docs/ui-stack.md: fixed 220 px)",
    ).toBeDefined();
    expect(["220px", "13.75rem"]).toContain(width);
  });

  it("the_sidebar_shows_the_product_name", async () => {
    const shell = await mountShell();

    expect(sidebarText(shell)).toContain("Keitora");
  });

  it("the_sidebar_is_always_visible", async () => {
    const shell = await mountShell();
    const { Sidebar, SidebarTrigger } = await import("@/components/ui/sidebar");

    expect(
      shell.wrapper.findComponent(Sidebar).props("collapsible"),
      "the sidebar is fixed (docs/ui-stack.md), so it does not collapse",
    ).toBe("none");
    expect(
      shell.wrapper.findComponent(SidebarTrigger).exists(),
      "a fixed sidebar has no control to hide it",
    ).toBe(false);

    for (const item of NAV_ITEMS) {
      await goTo(shell, item);
      expect(
        NAV_ITEMS.every((other) => navLink(shell, other).isConnected),
        `the four items should stay visible on the ${item.en} screen`,
      ).toBe(true);
    }
  });
});
