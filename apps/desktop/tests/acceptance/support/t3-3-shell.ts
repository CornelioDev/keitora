// SPDX-License-Identifier: MIT OR Apache-2.0

/**
 * Helpers shared by the T-3.3 acceptance tests (the app shell).
 *
 * They start the shell the way src/main.ts starts the app — vue-i18n, Pinia,
 * vue-query and the app's router installed on src/App.vue, with
 * `import.meta.env.DEV` false so that what production renders is what the
 * tests see — and they stand in for the browser APIs jsdom lacks but WebView2
 * has: `matchMedia` (with a `prefers-color-scheme` the test controls),
 * ResizeObserver and pointer capture, which reka-ui and VueUse call.
 *
 * The Tauri window is faked too: the window controls of the custom title bar
 * must go through `@tauri-apps/api`, and the tests assert which of its
 * methods they call.
 */

import { QueryClient, VueQueryPlugin } from "@tanstack/vue-query";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia } from "pinia";
import { expect, vi } from "vitest";
import type { Component } from "vue";
import type { Router } from "vue-router";

import App from "@/App.vue";
import { createAppI18n, type Locale } from "@/i18n";

/** The app's vue-i18n instance, as src/main.ts creates it. */
export type AppI18n = ReturnType<typeof createAppI18n>;

/**
 * The four screens of the shell: the route name and path the app uses, the
 * page component of docs/ui-stack.md ("Frontend layout": one SFC per route)
 * and the label people read, in both languages.
 *
 * The English labels come from the roadmap (T-3.3), the Spanish ones from
 * the MVP mockups (docs/assets/mockups), which are the visual reference.
 */
export const NAV_ITEMS = [
  {
    name: "timer",
    path: "/timer",
    page: "TimerPage.vue",
    en: "Timer",
    es: "Temporizador",
  },
  {
    name: "blocklists",
    path: "/blocklists",
    page: "BlocklistsPage.vue",
    en: "Blocklists",
    es: "Listas de bloqueo",
  },
  {
    name: "history",
    path: "/history",
    page: "HistoryPage.vue",
    en: "History",
    es: "Historial",
  },
  {
    name: "settings",
    path: "/settings",
    page: "SettingsPage.vue",
    en: "Settings",
    es: "Ajustes",
  },
] as const;

/** One of the four screens. */
export type NavItem = (typeof NAV_ITEMS)[number];

/** The three window controls of the custom title bar. */
export const WINDOW_CONTROLS = [
  "minimize",
  "toggle-maximize",
  "close",
] as const;

/** One of the three window controls. */
export type WindowControl = (typeof WINDOW_CONTROLS)[number];

// --- The app's router module -------------------------------------------------

/** What src/router/index.ts must export. */
export interface RouterModule {
  createAppRouter: () => Router;
}

const routerModules = import.meta.glob<RouterModule>(
  "../../src/router/index.ts",
);
const pageModules = import.meta.glob<{ default: Component }>(
  "../../src/pages/*.vue",
);

/** The app's router module (src/router/index.ts). */
export async function loadRouterModule(): Promise<RouterModule> {
  const load = routerModules["../../src/router/index.ts"];
  expect(
    load,
    "src/router/index.ts should exist (docs/ui-stack.md, 'Frontend layout')",
  ).toBeDefined();
  const module = await load();
  expect(
    typeof module.createAppRouter,
    "src/router/index.ts should export createAppRouter(), which builds the app's router",
  ).toBe("function");
  return module;
}

/** The page component of a screen (src/pages/<Name>Page.vue). */
export async function loadPage(item: NavItem): Promise<Component> {
  const load = pageModules[`../../src/pages/${item.page}`];
  expect(
    load,
    `src/pages/${item.page} should exist (docs/ui-stack.md, 'Frontend layout')`,
  ).toBeDefined();
  return (await load()).default;
}

// --- Stand-ins for browser APIs jsdom lacks ----------------------------------

interface ChangeEvent {
  matches: boolean;
  media: string;
}

type Listener = EventListenerOrEventListenerObject;

let prefersDark = false;
const mediaQueries = new Set<FakeMediaQueryList>();

/** Whether a media query matches: only `prefers-color-scheme` ever does. */
function queryMatches(media: string): boolean {
  if (/prefers-color-scheme\s*:\s*dark/i.test(media)) return prefersDark;
  if (/prefers-color-scheme\s*:\s*light/i.test(media)) return !prefersDark;
  // The window is a desktop window: no `max-width` query matches.
  return false;
}

class FakeMediaQueryList {
  matches: boolean;
  onchange: ((event: ChangeEvent) => void) | null = null;
  private readonly listeners = new Set<Listener>();

  constructor(readonly media: string) {
    this.matches = queryMatches(media);
  }

  addEventListener(type: string, listener: Listener, _options?: unknown): void {
    if (type === "change") this.listeners.add(listener);
  }

  removeEventListener(
    type: string,
    listener: Listener,
    _options?: unknown,
  ): void {
    if (type === "change") this.listeners.delete(listener);
  }

  /** The old API, which VueUse and reka-ui fall back to. */
  addListener(listener: Listener): void {
    this.listeners.add(listener);
  }

  removeListener(listener: Listener): void {
    this.listeners.delete(listener);
  }

  dispatchEvent(): boolean {
    return true;
  }

  /** Re-evaluates the query and tells its listeners when it changed. */
  refresh(): void {
    const next = queryMatches(this.media);
    if (next === this.matches) return;
    this.matches = next;
    const event = { matches: next, media: this.media };
    for (const listener of [...this.listeners]) {
      if (typeof listener === "function") {
        listener(event as unknown as Event);
      } else {
        listener.handleEvent(event as unknown as Event);
      }
    }
    this.onchange?.(event);
  }
}

/** What the OS asks for; changing it notifies the app, as Windows would. */
export function setPrefersDark(value: boolean): void {
  prefersDark = value;
  for (const query of mediaQueries) query.refresh();
}

function defineIfMissing(target: object, key: string, value: unknown): void {
  if (!(key in target)) {
    Object.defineProperty(target, key, {
      configurable: true,
      writable: true,
      value,
    });
  }
}

/** Adds the browser APIs WebView2 has and jsdom does not. */
export function installBrowserApis(): void {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (media: string) => {
      const query = new FakeMediaQueryList(media);
      mediaQueries.add(query);
      return query as unknown as MediaQueryList;
    },
  });
  class ResizeObserverStandIn {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  defineIfMissing(globalThis, "ResizeObserver", ResizeObserverStandIn);
  defineIfMissing(Element.prototype, "scrollIntoView", () => {});
  defineIfMissing(Element.prototype, "hasPointerCapture", () => false);
  defineIfMissing(Element.prototype, "setPointerCapture", () => {});
  defineIfMissing(Element.prototype, "releasePointerCapture", () => {});
}

// --- The Tauri window --------------------------------------------------------

/**
 * The window the app runs in. The title bar's controls must reach it through
 * `@tauri-apps/api`, which the tests mock with this object; every method is
 * a spy, so a test can assert which ones a control uses and which ones it
 * never touches (each Tauri method needs its own permission, and the
 * maintainer approved exactly four).
 */
export const appWindow = {
  minimize: vi.fn(async (): Promise<void> => {}),
  toggleMaximize: vi.fn(async (): Promise<void> => {}),
  maximize: vi.fn(async (): Promise<void> => {}),
  unmaximize: vi.fn(async (): Promise<void> => {}),
  close: vi.fn(async (): Promise<void> => {}),
  destroy: vi.fn(async (): Promise<void> => {}),
  hide: vi.fn(async (): Promise<void> => {}),
  show: vi.fn(async (): Promise<void> => {}),
  startDragging: vi.fn(async (): Promise<void> => {}),
  setDecorations: vi.fn(async (): Promise<void> => {}),
  theme: vi.fn(async (): Promise<string> => "light"),
  isMaximized: vi.fn(async (): Promise<boolean> => false),
  onResized: vi.fn(async (): Promise<() => void> => () => {}),
  listen: vi.fn(async (): Promise<() => void> => () => {}),
  label: "main",
};

/** Forgets the calls recorded on the window between tests. */
export function resetAppWindow(): void {
  for (const value of Object.values(appWindow)) {
    if (vi.isMockFunction(value)) value.mockClear();
  }
}

// --- Mounting the shell ------------------------------------------------------

/** How to start the shell. */
export interface ShellOptions {
  /** Where the app starts; by default wherever the app itself starts. */
  path?: string;
  /** The language the app runs in; English by default. */
  locale?: Locale;
  /** What the OS asks for; the light theme by default. */
  prefersDark?: boolean;
}

/** A running shell. */
export interface Shell {
  wrapper: VueWrapper;
  /** The element the shell is rendered into, as `#app` is in index.html. */
  host: HTMLElement;
  router: Router;
  i18n: AppI18n;
}

let mounted: VueWrapper | undefined;

/** Starts the app shell with the plugins src/main.ts installs. */
export async function mountShell(options: ShellOptions = {}): Promise<Shell> {
  installBrowserApis();
  setPrefersDark(options.prefersDark ?? false);
  // Production builds replace `import.meta.env.DEV` with false; the shell is
  // what they render (src/App.vue, T-0.8).
  vi.stubEnv("DEV", false);

  const { createAppRouter } = await loadRouterModule();
  const router = createAppRouter();
  if (options.path !== undefined) {
    await router.push(options.path);
  }
  await router.isReady();

  const i18n = createAppI18n(options.locale ?? "en");
  // No retries: a failing request must show at once, not after a wait.
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  const host = document.createElement("div");
  document.body.appendChild(host);
  const wrapper = mount(App, {
    attachTo: host,
    global: {
      plugins: [i18n, createPinia(), [VueQueryPlugin, { queryClient }], router],
    },
  });
  mounted = wrapper;
  await flushPromises();

  return { wrapper, host, router, i18n };
}

/** Undoes what `mountShell` set up. Call it in `afterEach`. */
export function cleanupShell(): void {
  mounted?.unmount();
  mounted = undefined;
  mediaQueries.clear();
  prefersDark = false;
  document.body.innerHTML = "";
  document.documentElement.className = "";
  document.documentElement.removeAttribute("style");
  resetAppWindow();
  vi.unstubAllEnvs();
}

/** Goes to a screen by its route name and lets the shell settle. */
export async function goTo(shell: Shell, item: NavItem): Promise<void> {
  await shell.router.push({ name: item.name });
  await flushPromises();
}

// --- Looking at the shell ----------------------------------------------------

/** Every element the shell renders. */
export function elementsOf(shell: Shell): HTMLElement[] {
  return [...shell.host.querySelectorAll<HTMLElement>("*")];
}

/** The first element of the shell matching a CSS selector, or null. */
export function findIn(shell: Shell, selector: string): HTMLElement | null {
  return shell.host.querySelector<HTMLElement>(selector);
}

/** The `<main>` element, which holds the screen the router shows. */
export function mainArea(shell: Shell): HTMLElement {
  const main = findIn(shell, "main");
  expect(
    main,
    "the shell should render the screen inside a <main> element (WCAG 2.2, 1.3.1)",
  ).not.toBeNull();
  return main as HTMLElement;
}

/** The name a screen shows as the heading of the main area (MVP mockups). */
export function mainHeading(shell: Shell): string {
  const headings = mainArea(shell).querySelectorAll("h1");
  expect(
    headings.length,
    "the screen should show its name as the one heading (<h1>) of the main area",
  ).toBe(1);
  return (headings[0].textContent ?? "").replace(/\s+/g, " ").trim();
}

/**
 * The name a screen reader announces for an element: its `aria-label`, the
 * text of the elements its `aria-labelledby` points at, its own text, or its
 * `title` (WCAG 2.2 AA, 4.1.2).
 */
export function accessibleName(element: Element): string {
  const label = element.getAttribute("aria-label")?.trim() ?? "";
  if (label !== "") return label;
  const labelledBy = (element.getAttribute("aria-labelledby") ?? "")
    .split(/\s+/)
    .filter((id) => id !== "")
    .map((id) => element.ownerDocument.getElementById(id)?.textContent ?? "")
    .join(" ")
    .trim();
  if (labelledBy !== "") return labelledBy;
  const text = (element.textContent ?? "").replace(/\s+/g, " ").trim();
  if (text !== "") return text;
  return element.getAttribute("title")?.trim() ?? "";
}

/** The navigation link of a screen, found by the path it points at. */
export function navLink(shell: Shell, item: NavItem): HTMLAnchorElement {
  const links = elementsOf(shell).filter(
    (element): element is HTMLAnchorElement =>
      element.tagName === "A" && element.getAttribute("href") === item.path,
  );
  expect(
    links.length,
    `the sidebar should have one link to ${item.path} (${item.en})`,
  ).toBe(1);
  return links[0];
}

/** The button of a window control, marked `data-window-control`. */
export function windowControl(
  shell: Shell,
  control: WindowControl,
): HTMLElement {
  const buttons = elementsOf(shell).filter(
    (element) => element.dataset.windowControl === control,
  );
  expect(
    buttons.length,
    `the title bar should have one control marked data-window-control="${control}"`,
  ).toBe(1);
  return buttons[0];
}

/** Everything a keyboard reaches in the shell: links and buttons. */
export function interactiveElements(shell: Shell): HTMLElement[] {
  return elementsOf(shell).filter(
    (element) =>
      (element.tagName === "A" && element.hasAttribute("href")) ||
      element.tagName === "BUTTON",
  );
}

/** A short description of an element, for failure messages. */
export function describeElement(element: Element): string {
  const attributes = [...element.attributes]
    .filter((attribute) => attribute.name !== "class")
    .map((attribute) => `${attribute.name}="${attribute.value}"`)
    .join(" ");
  return `<${element.tagName.toLowerCase()} ${attributes}>`.replace(
    /\s+>/,
    ">",
  );
}
