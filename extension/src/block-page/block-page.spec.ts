// SPDX-License-Identifier: MIT OR Apache-2.0
// @vitest-environment jsdom

// Unit tests of the block page as it runs: who it listens to, what it shows
// in each state, and which messages it can ask `chrome.i18n` for. They run the
// real page against a small `chrome` of their own, so they do not depend on
// the acceptance tests' support code.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BLOCK_STATE, type BlockState } from "./block-state";

// Paths, not `new URL("<relative>", import.meta.url)`: under jsdom, Vite
// rewrites that expression into a dev-server address `readFileSync` cannot
// open.
const here = dirname(fileURLToPath(import.meta.url));
const PAGE_HTML = readFileSync(join(here, "index.html"), "utf-8");

const EXTENSION_ID = "keitoraunittestsxxxxxxxxxxxxxxxx";
const SERVICE_WORKER = { id: EXTENSION_ID };
const NOW = new Date(2026, 0, 2, 13, 55, 28).getTime();
const BLOCKED = `?url=${encodeURIComponent("https://news.example.org/a")}`;

type Listener = (
  message: unknown,
  sender: unknown,
  sendResponse: (response?: unknown) => void,
) => unknown;

/** 14:32 left of a strict focus phase that ends at 14:10. */
function strictState(overrides: Partial<BlockState> = {}): BlockState {
  return {
    phase: "focus",
    cycle: 2,
    cycles: 4,
    remainingMs: 872_000,
    asOf: NOW,
    strict: true,
    strictEndsAt: new Date(2026, 0, 2, 14, 10, 0).getTime(),
    ...overrides,
  };
}

function relaxedState(overrides: Partial<BlockState> = {}): BlockState {
  return strictState({ strict: false, strictEndsAt: null, ...overrides });
}

/** The `chrome` the page is given, answering as each test configures. */
class FakeChrome {
  readonly keys: string[] = [];
  readonly listeners: Listener[] = [];
  /** What `sendMessage` resolves to; a function to decide when it does. */
  answer: () => Promise<unknown> = () =>
    Promise.resolve({ type: BLOCK_STATE, state: null });

  readonly api = {
    runtime: {
      id: EXTENSION_ID,
      sendMessage: (_message: unknown) => this.answer(),
      onMessage: {
        addListener: (listener: Listener) => {
          this.listeners.push(listener);
        },
      },
    },
    i18n: {
      getMessage: (key: string, substitutions: string[] = []) => {
        this.keys.push(key);
        return substitutions.length === 0
          ? `[[${key}]]`
          : `[[${key}:${substitutions.join("|")}]]`;
      },
      getUILanguage: () => "en",
    },
  };

  answerWith(state: BlockState | null): void {
    this.answer = () => Promise.resolve({ type: BLOCK_STATE, state });
  }

  /** Delivers a message to the page as Chrome would, from `sender`. */
  deliver(message: unknown, sender: unknown): void {
    for (const listener of this.listeners) {
      listener(message, sender, () => {});
    }
  }
}

/** Runs the page, with its markup and its address, against `chrome`. */
async function loadPage(chrome: FakeChrome, search = BLOCKED): Promise<void> {
  vi.stubGlobal("chrome", chrome.api);
  window.history.replaceState(null, "", `/block-page/index.html${search}`);
  const parsed = new DOMParser().parseFromString(PAGE_HTML, "text/html");
  document.replaceChild(
    document.importNode(parsed.documentElement, true),
    document.documentElement,
  );
  vi.resetModules();
  await import("./block-page");
  await vi.advanceTimersByTimeAsync(0);
}

function text(): string {
  return (document.body.textContent ?? "").replace(/\s+/g, " ").trim();
}

/** Whether a part of the page is hidden; the page toggles the attribute. */
function isHidden(id: string): boolean {
  return document.getElementById(id)?.hasAttribute("hidden") ?? true;
}

let chrome: FakeChrome;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  chrome = new FakeChrome();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("who the page listens to", () => {
  beforeEach(async () => {
    chrome.answerWith(strictState());
    await loadPage(chrome);
    expect(text()).toContain("14:32");
    expect(isHidden("strict-notice")).toBe(false);
  });

  it("ignores another page's request, which carries no state", async () => {
    chrome.deliver({ type: BLOCK_STATE }, SERVICE_WORKER);
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("14:32");
    expect(isHidden("strict-notice")).toBe(false);
    expect(chrome.keys).not.toContain("blockPageNoData");
  });

  it("ignores a state from a page of the extension in a tab", async () => {
    // Another block page, which a hostile site can open in a frame.
    chrome.deliver(
      { type: BLOCK_STATE, state: null },
      { id: EXTENSION_ID, tab: { id: 3 }, frameId: 1 },
    );
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("14:32");
    expect(isHidden("strict-notice")).toBe(false);
  });

  it("ignores a state from another extension", async () => {
    chrome.deliver(
      { type: BLOCK_STATE, state: null },
      { id: "anotherextensionxxxxxxxxxxxxxxxx" },
    );
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("14:32");
  });

  it("ignores a state with no sender", async () => {
    chrome.deliver({ type: BLOCK_STATE, state: null }, undefined);
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("14:32");
  });

  it("shows a state the service worker pushes", async () => {
    chrome.deliver(
      { type: BLOCK_STATE, state: relaxedState({ remainingMs: 60_000 }) },
      SERVICE_WORKER,
    );
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("01:00");
    expect(text()).not.toContain("14:32");
    expect(isHidden("strict-notice")).toBe(true);
  });

  it("shows that the service worker says there is no session", async () => {
    chrome.deliver({ type: BLOCK_STATE, state: null }, SERVICE_WORKER);
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).not.toContain("14:32");
    expect(chrome.keys).toContain("blockPageNoData");
  });
});

describe("the page's own answer", () => {
  /** Holds the answer back until the test lets it through. */
  function holdAnswer(state: BlockState): () => void {
    let release = (): void => {};
    chrome.answer = () =>
      new Promise((resolve) => {
        release = () => {
          resolve({ type: BLOCK_STATE, state });
        };
      });
    return () => {
      release();
    };
  }

  it("is not dropped because of a stray request that arrived first", async () => {
    const release = holdAnswer(strictState());
    await loadPage(chrome);

    chrome.deliver({ type: BLOCK_STATE }, { id: EXTENSION_ID, tab: { id: 2 } });
    chrome.deliver({ type: BLOCK_STATE }, SERVICE_WORKER);
    release();
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("14:32");
    expect(isHidden("strict-notice")).toBe(false);
  });

  it("is not dropped because of a state from a framed page", async () => {
    const release = holdAnswer(strictState());
    await loadPage(chrome);

    chrome.deliver(
      { type: BLOCK_STATE, state: null },
      { id: EXTENSION_ID, tab: { id: 2 } },
    );
    release();
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("14:32");
  });

  it("gives way to a newer state the service worker pushed meanwhile", async () => {
    const release = holdAnswer(strictState());
    await loadPage(chrome);

    chrome.deliver(
      { type: BLOCK_STATE, state: relaxedState({ remainingMs: 60_000 }) },
      SERVICE_WORKER,
    );
    release();
    await vi.advanceTimersByTimeAsync(0);

    expect(text()).toContain("01:00");
    expect(text()).not.toContain("14:32");
  });
});

describe("the strict notice", () => {
  it.each(["focus", "short_break", "long_break"] as const)(
    "never appears outside strict mode (%s)",
    async (phase) => {
      chrome.answerWith(relaxedState({ phase }));
      await loadPage(chrome);
      await vi.advanceTimersByTimeAsync(5_000);

      expect(isHidden("strict-notice")).toBe(true);
      expect(document.getElementById("strict-text")?.textContent).toBe("");
      expect(chrome.keys).not.toContain("blockPageStrictUntil");
      expect(chrome.keys).not.toContain("blockPageStrictLocked");
    },
  );

  it("never appears outside strict mode, even with an end time", async () => {
    chrome.answerWith(
      relaxedState({ strictEndsAt: new Date(2026, 0, 2, 14, 10).getTime() }),
    );
    await loadPage(chrome);

    expect(isHidden("strict-notice")).toBe(true);
    expect(text()).not.toMatch(/Strict/);
  });

  it("goes away when a pushed state is not strict", async () => {
    chrome.answerWith(strictState());
    await loadPage(chrome);
    expect(isHidden("strict-notice")).toBe(false);

    chrome.deliver(
      { type: BLOCK_STATE, state: relaxedState() },
      SERVICE_WORKER,
    );
    await vi.advanceTimersByTimeAsync(0);

    expect(isHidden("strict-notice")).toBe(true);
    expect(document.getElementById("strict-text")?.textContent).toBe("");
  });

  it("says the session is locked when it has no end time", async () => {
    chrome.answerWith(strictState({ strictEndsAt: null }));
    await loadPage(chrome);

    expect(isHidden("strict-notice")).toBe(false);
    expect(chrome.keys).toContain("blockPageStrictLocked");
    expect(chrome.keys).not.toContain("blockPageStrictUntil");
  });
});

describe("the explanation", () => {
  it("is shown while there is a session", async () => {
    chrome.answerWith(relaxedState());
    await loadPage(chrome);

    expect(isHidden("explanation")).toBe(false);
  });

  it("takes no room when there is no session", async () => {
    chrome.answerWith(null);
    await loadPage(chrome);

    expect(isHidden("explanation")).toBe(true);
    expect(document.getElementById("explanation")?.textContent).toBe("");
  });

  it("is hidden until the first answer arrives", () => {
    const parsed = new DOMParser().parseFromString(PAGE_HTML, "text/html");

    expect(parsed.getElementById("explanation")?.hasAttribute("hidden")).toBe(
      true,
    );
  });
});

describe("the messages the page uses", () => {
  interface LocaleMessage {
    message: string;
    description?: string;
  }

  function messages(locale: string): Record<string, LocaleMessage> {
    const path = join(here, "..", "_locales", locale, "messages.json");
    return JSON.parse(readFileSync(path, "utf-8")) as Record<
      string,
      LocaleMessage
    >;
  }

  /** One tick of the page's clock, so that a redraw is counted as well. */
  const TICK = 1_000;

  /** Every key the page asks for, across every state it can be in. */
  async function keysOfEveryState(): Promise<Set<string>> {
    const states: (BlockState | null)[] = [
      strictState(),
      strictState({ strictEndsAt: null }),
      relaxedState(),
      relaxedState({ phase: "short_break" }),
      relaxedState({ phase: "long_break" }),
      null,
    ];
    const addresses = [BLOCKED, "", "?url=javascript%3Aalert(1)"];
    for (const search of addresses) {
      for (const state of states) {
        chrome.answerWith(state);
        await loadPage(chrome, search);
        await vi.advanceTimersByTimeAsync(TICK);
      }
      chrome.answer = () => Promise.reject(new Error("no receiving end"));
      await loadPage(chrome, search);
    }
    return new Set(chrome.keys);
  }

  it("covers the messages of the rarer states", async () => {
    const keys = await keysOfEveryState();

    for (const key of [
      "blockPageStrictLocked",
      "blockPageStrictUntil",
      "blockPageHeadingUnknownSite",
      "blockPageHeading",
      "blockPageNoData",
      "blockPagePhaseFocus",
      "blockPagePhaseShortBreak",
      "blockPagePhaseLongBreak",
    ]) {
      expect(keys, key).toContain(key);
    }
  });

  it.each(["en", "es"])(
    "finds every message it uses in %s, never empty",
    async (locale) => {
      const keys = await keysOfEveryState();
      const defined = messages(locale);

      for (const key of keys) {
        expect(defined, `${locale} is missing "${key}"`).toHaveProperty(key);
        expect(defined[key]?.message.trim(), `${locale} ${key}`).not.toBe("");
      }
    },
  );

  it("describes every message it uses for translators", async () => {
    const keys = await keysOfEveryState();
    const english = messages("en");

    for (const key of keys) {
      expect(english[key]?.description?.trim(), key).toBeTruthy();
    }
  });
});
