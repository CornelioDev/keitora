// SPDX-License-Identifier: MIT OR Apache-2.0

import { mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { defineComponent, h, type PropType } from "vue";

import { useTheme } from "./useTheme";

/**
 * A stand-in for `matchMedia`, which jsdom does not implement: it answers
 * `(prefers-color-scheme: dark)` with what the test asks for and notifies
 * its listeners when that changes, as the webview does when Windows'
 * theme changes.
 */
class FakeMediaQueryList {
  matches: boolean;
  private readonly listeners = new Set<(event: Event) => void>();

  constructor(
    readonly media: string,
    dark: boolean,
  ) {
    this.matches = /dark/.test(media) ? dark : !dark;
  }

  addEventListener(type: string, listener: (event: Event) => void): void {
    if (type === "change") this.listeners.add(listener);
  }

  removeEventListener(type: string, listener: (event: Event) => void): void {
    if (type === "change") this.listeners.delete(listener);
  }

  set(matches: boolean): void {
    if (matches === this.matches) return;
    this.matches = matches;
    for (const listener of [...this.listeners]) {
      listener({ matches } as unknown as Event);
    }
  }
}

let queries: FakeMediaQueryList[] = [];

function installMatchMedia(dark: boolean): void {
  queries = [];
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (media: string) => {
      const query = new FakeMediaQueryList(media, dark);
      queries.push(query);
      return query as unknown as MediaQueryList;
    },
  });
}

/** Tells the app that the operating system switched theme. */
function osSwitchesToDark(dark: boolean): void {
  for (const query of queries) {
    query.set(/dark/.test(query.media) ? dark : !dark);
  }
}

/**
 * A component that does nothing but follow the system's theme, on the
 * element given as `root`, or on `<html>` without one.
 */
const Themed = defineComponent({
  props: {
    root: { type: Object as PropType<HTMLElement>, default: undefined },
  },
  setup(props) {
    useTheme(props.root);
    return () => h("div");
  },
});

afterEach(() => {
  document.documentElement.className = "";
});

describe("useTheme", () => {
  beforeEach(() => {
    installMatchMedia(false);
  });

  it("starts in the theme the system asks for", () => {
    installMatchMedia(true);

    mount(Themed);

    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("leaves the light theme alone when the system asks for it", () => {
    mount(Themed);

    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("follows the system both ways while it runs", () => {
    mount(Themed);

    osSwitchesToDark(true);
    expect(document.documentElement.classList.contains("dark")).toBe(true);

    osSwitchesToDark(false);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("puts the theme on the element it is given", () => {
    installMatchMedia(true);
    const root = document.createElement("div");

    mount(Themed, { props: { root } });

    expect(root.classList.contains("dark")).toBe(true);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
