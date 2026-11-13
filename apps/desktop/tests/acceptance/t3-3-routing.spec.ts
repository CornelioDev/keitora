// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-3.3, the routing half of the app shell: "App shell:
// shadcn-vue `Sidebar` (Timer, Blocklists, History, Settings), custom title
// bar, Vue Router, theme following the OS". Criteria derived from the task
// line, docs/ui-stack.md ("Routing | Vue Router (memory history)", "Frontend
// layout": `router/` and `pages/`) and the MVP mockups (every screen shows
// its name as the heading of the main area).
//
//  - AC-1 the app has one route per screen: timer, blocklists, history and
//    settings, at /timer, /blocklists, /history and /settings
//      the_app_has_a_route_for_each_of_the_four_screens
//      every_route_renders_the_page_component_of_its_screen
//  - AC-2 the router uses memory history, so no navigation touches the URL
//    of the webview (there is no address bar)
//      the_router_is_built_on_memory_history
//      navigating_does_not_change_the_address_of_the_webview
//  - AC-3 the app starts on the timer screen
//      the_app_starts_on_the_timer_screen
//  - AC-4 every screen is reachable, and the main area shows the screen's
//    name as its heading
//      navigating_to_a_screen_shows_it_in_the_main_area
//      going_back_returns_to_the_previous_screen
//  - AC-5 an address that is not a screen lands on the timer screen instead
//    of an empty window (failure path)
//      an_unknown_address_lands_on_the_timer_screen
//
// The app's router module is src/router/index.ts and exports
// `createAppRouter()`, so that each test gets its own router, as src/main.ts
// does when it starts the app.

import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { IDLE_STATE, SETTINGS, tauri } from "./support/t3-2-tauri-fake";
import {
  NAV_ITEMS,
  cleanupShell,
  goTo,
  loadPage,
  loadRouterModule,
  mainHeading,
  mountShell,
} from "./support/t3-3-shell";

// The shell runs in a Tauri window; the data layer (T-3.2) and the window
// controls (the title bar) are the only ways out of the webview.
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

// Which history vue-router is built on is the point of AC-2, so the two
// histories the app must not use are spied on as well.
vi.mock("vue-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("vue-router")>();
  return {
    ...actual,
    createMemoryHistory: vi.fn(actual.createMemoryHistory),
    createWebHistory: vi.fn(actual.createWebHistory),
    createWebHashHistory: vi.fn(actual.createWebHashHistory),
  };
});

const TIMER = NAV_ITEMS[0];

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

describe("routing (T-3.3)", () => {
  it("the_app_has_a_route_for_each_of_the_four_screens", async () => {
    const { createAppRouter } = await loadRouterModule();
    const router = createAppRouter();

    for (const item of NAV_ITEMS) {
      const byName = router.resolve({ name: item.name });
      expect(byName.path, `the route named "${item.name}"`).toBe(item.path);
      const byPath = router.resolve(item.path);
      expect(byPath.name, `the route at ${item.path}`).toBe(item.name);
      expect(
        byPath.matched.length,
        `${item.path} should be matched by a route`,
      ).toBeGreaterThan(0);
    }
  });

  it("every_route_renders_the_page_component_of_its_screen", async () => {
    const { createAppRouter } = await loadRouterModule();
    const router = createAppRouter();

    for (const item of NAV_ITEMS) {
      const page = await loadPage(item);
      const record = router.resolve({ name: item.name }).matched.at(-1);
      const component = record?.components?.default;
      expect(
        component,
        `the route "${item.name}" should render a page`,
      ).toBeDefined();
      const resolved =
        typeof component === "function"
          ? ((await (component as () => Promise<{ default: unknown }>)())
              .default ?? component)
          : component;
      expect(
        resolved,
        `the route "${item.name}" should render src/pages/${item.page}`,
      ).toBe(page);
    }
  });

  it("the_router_is_built_on_memory_history", async () => {
    const { createMemoryHistory, createWebHistory, createWebHashHistory } =
      await import("vue-router");
    const { createAppRouter } = await loadRouterModule();

    createAppRouter();

    expect(
      vi.mocked(createMemoryHistory),
      "the app has no address bar, so the router uses createMemoryHistory() (docs/ui-stack.md)",
    ).toHaveBeenCalled();
    expect(vi.mocked(createWebHistory)).not.toHaveBeenCalled();
    expect(vi.mocked(createWebHashHistory)).not.toHaveBeenCalled();
  });

  it("navigating_does_not_change_the_address_of_the_webview", async () => {
    const before = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    const shell = await mountShell();

    for (const item of NAV_ITEMS) {
      await goTo(shell, item);
      expect(shell.router.currentRoute.value.path).toBe(item.path);
      expect(
        `${window.location.pathname}${window.location.search}${window.location.hash}`,
        "memory history keeps the navigation out of the webview's address",
      ).toBe(before);
    }
  });

  it("the_app_starts_on_the_timer_screen", async () => {
    const shell = await mountShell();

    expect(shell.router.currentRoute.value.name).toBe(TIMER.name);
    expect(shell.router.currentRoute.value.path).toBe(TIMER.path);
    expect(mainHeading(shell).toLowerCase()).toBe(TIMER.en.toLowerCase());
  });

  it("navigating_to_a_screen_shows_it_in_the_main_area", async () => {
    const shell = await mountShell();

    for (const item of NAV_ITEMS) {
      await goTo(shell, item);

      expect(shell.router.currentRoute.value.path).toBe(item.path);
      expect(
        mainHeading(shell).toLowerCase(),
        `the main area should show the ${item.en} screen`,
      ).toBe(item.en.toLowerCase());
    }
  });

  it("going_back_returns_to_the_previous_screen", async () => {
    const shell = await mountShell();
    await goTo(shell, NAV_ITEMS[3]);
    await goTo(shell, NAV_ITEMS[2]);

    shell.router.back();
    await flushPromises();

    expect(shell.router.currentRoute.value.name).toBe(NAV_ITEMS[3].name);
    expect(mainHeading(shell).toLowerCase()).toBe(
      NAV_ITEMS[3].en.toLowerCase(),
    );
  });

  it("an_unknown_address_lands_on_the_timer_screen", async () => {
    const shell = await mountShell({ path: "/not-a-screen" });

    expect(
      shell.router.currentRoute.value.name,
      "an address that is not a screen should land on the timer screen, not on an empty window",
    ).toBe(TIMER.name);
    expect(mainHeading(shell).toLowerCase()).toBe(TIMER.en.toLowerCase());
  });
});
