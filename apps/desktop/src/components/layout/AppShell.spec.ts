// SPDX-License-Identifier: MIT OR Apache-2.0

import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createAppI18n, type Locale } from "@/i18n";
import { createAppRouter } from "@/router";

import AppShell from "./AppShell.vue";

/** The window the title bar acts on, with every method a spy. */
const appWindow = vi.hoisted(() => ({
  minimize: vi.fn(async (): Promise<void> => {}),
  toggleMaximize: vi.fn(async (): Promise<void> => {}),
  close: vi.fn(async (): Promise<void> => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => appWindow,
}));

/** The screens of the shell: route name, path and label in both languages. */
const SCREENS = [
  { name: "timer", path: "/timer", en: "Timer", es: "Temporizador" },
  {
    name: "blocklists",
    path: "/blocklists",
    en: "Blocklists",
    es: "Listas de bloqueo",
  },
  { name: "history", path: "/history", en: "History", es: "Historial" },
  { name: "settings", path: "/settings", en: "Settings", es: "Ajustes" },
] as const;

/** jsdom has no matchMedia; nothing the shell asks for matches. */
function installMatchMedia(): void {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (media: string) => ({
      media,
      matches: false,
      addEventListener: () => {},
      removeEventListener: () => {},
    }),
  });
}

let wrapper: VueWrapper | undefined;

async function mountShell(locale: Locale = "en"): Promise<{
  host: HTMLElement;
  router: ReturnType<typeof createAppRouter>;
}> {
  installMatchMedia();
  const router = createAppRouter();
  const host = document.createElement("div");
  document.body.appendChild(host);
  // As in src/main.ts, the router makes its first navigation when it is
  // installed, so it is only ready after mounting.
  wrapper = mount(AppShell, {
    attachTo: host,
    global: { plugins: [createAppI18n(locale), createPinia(), router] },
  });
  await router.isReady();
  await flushPromises();
  return { host, router };
}

function link(host: HTMLElement, path: string): HTMLAnchorElement {
  const found = host.querySelector<HTMLAnchorElement>(`a[href="${path}"]`);
  expect(found, `a link to ${path}`).not.toBeNull();
  return found as HTMLAnchorElement;
}

function control(host: HTMLElement, name: string): HTMLElement {
  const found = host.querySelector<HTMLElement>(
    `[data-window-control=${name}]`,
  );
  expect(found, `the ${name} control`).not.toBeNull();
  return found as HTMLElement;
}

function follows(first: Element, second: Element): boolean {
  return Boolean(
    first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING,
  );
}

beforeEach(() => {
  installMatchMedia();
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  document.body.innerHTML = "";
  document.documentElement.className = "";
  vi.clearAllMocks();
});

describe("AppShell", () => {
  it("lays out the title bar, the sidebar and the screen in that order", async () => {
    const { host } = await mountShell();

    const dragRegion = host.querySelector("[data-tauri-drag-region]");
    const nav = host.querySelector("nav");
    const main = host.querySelector("main");

    expect(dragRegion).not.toBeNull();
    expect(host.querySelectorAll("nav")).toHaveLength(1);
    expect(host.querySelectorAll("main")).toHaveLength(1);
    expect(follows(dragRegion as Element, nav as Element)).toBe(true);
    expect(follows(nav as Element, main as Element)).toBe(true);
  });

  it("gives the sidebar its fixed width and the product name", async () => {
    const { host } = await mountShell();

    const widths = [...host.querySelectorAll<HTMLElement>("*")]
      .map((element) => element.style.getPropertyValue("--sidebar-width"))
      .filter((value) => value !== "");

    expect(widths[0]).toBe("220px");
    expect(host.textContent).toContain("Keitora");
  });

  it("lists the four screens as links, in order", async () => {
    const { host } = await mountShell();

    const links = SCREENS.map((screen) => link(host, screen.path));
    for (const [index, screen] of SCREENS.entries()) {
      expect(links[index].textContent?.trim()).toBe(screen.en);
      expect(links[index].closest("nav")).not.toBeNull();
      if (index > 0) {
        expect(follows(links[index - 1], links[index])).toBe(true);
      }
    }
  });

  it("opens a screen when its link is clicked and marks it as current", async () => {
    const { host, router } = await mountShell();

    for (const screen of SCREENS) {
      link(host, screen.path).click();
      await flushPromises();

      expect(router.currentRoute.value.name).toBe(screen.name);
      expect(link(host, screen.path).getAttribute("aria-current")).toBe("page");
      const current = SCREENS.filter(
        (other) => link(host, other.path).getAttribute("aria-current") !== null,
      );
      expect(current).toHaveLength(1);
      expect(host.querySelector("main")?.querySelectorAll("h1")).toHaveLength(
        1,
      );
      expect(host.querySelector("main h1")?.textContent?.trim()).toBe(
        screen.en,
      );
    }
  });

  it("names the screens and the window controls in Spanish too", async () => {
    const { host } = await mountShell("es");

    for (const screen of SCREENS) {
      expect(link(host, screen.path).textContent?.trim()).toBe(screen.es);
    }
    expect(control(host, "close").getAttribute("aria-label")).toBe("Cerrar");
  });

  it("acts on the window only when a control is used", async () => {
    const { host } = await mountShell();

    expect(appWindow.minimize).not.toHaveBeenCalled();
    expect(appWindow.toggleMaximize).not.toHaveBeenCalled();
    expect(appWindow.close).not.toHaveBeenCalled();

    control(host, "minimize").click();
    await flushPromises();
    expect(appWindow.minimize).toHaveBeenCalledTimes(1);

    control(host, "toggle-maximize").click();
    await flushPromises();
    expect(appWindow.toggleMaximize).toHaveBeenCalledTimes(1);

    control(host, "close").click();
    await flushPromises();
    expect(appWindow.close).toHaveBeenCalledTimes(1);
  });

  it("keeps the window controls out of the drag region", async () => {
    const { host } = await mountShell();

    for (const name of ["minimize", "toggle-maximize", "close"]) {
      const button = control(host, name);
      expect(button.tagName).toBe("BUTTON");
      expect(button.getAttribute("type")).toBe("button");
      expect(button.hasAttribute("data-tauri-drag-region")).toBe(false);
      expect(button.querySelectorAll("[data-tauri-drag-region]")).toHaveLength(
        0,
      );
    }
  });

  it("gives every control a name and a visible focus ring", async () => {
    const { host } = await mountShell();

    const controls = [...host.querySelectorAll<HTMLElement>("a[href], button")];
    expect(controls.length).toBeGreaterThan(0);
    for (const element of controls) {
      const name =
        element.getAttribute("aria-label")?.trim() ??
        element.textContent?.trim() ??
        "";
      expect(name, element.outerHTML).not.toBe("");
      expect(/focus-visible:|focus:/.test(element.className)).toBe(true);
    }
  });
});
