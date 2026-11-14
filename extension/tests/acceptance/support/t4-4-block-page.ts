// SPDX-License-Identifier: MIT OR Apache-2.0

// Support for the T-4.4 acceptance tests: a stand-in for the `chrome` APIs the
// block page uses, and a loader that runs the real page against it.
//
// The page's data source does not exist yet: the service worker learns the
// session's state in T-4.2, through the native messaging host of T-4.1. These
// tests therefore pin the contract between the page and the service worker and
// fake the service worker's side of it, the same way the T-3.2 tests fake the
// Tauri boundary (apps/desktop/tests/acceptance/support/t3-2-tauri-fake.ts).
//
// The contract, which T-4.2 implements on the service worker's side:
//
//   - The blocked address reaches the page in its own URL, as the `url` query
//     parameter (`block-page/index.html?url=<the blocked URL>`), percent
//     encoded or not. The page shows the host of that URL and nothing else of
//     it, and only for `http` and `https` (keitora-core blocks no other
//     scheme).
//   - When it opens, the page asks the service worker for the session's state
//     with `chrome.runtime.sendMessage({ type: "keitora:block-state" })`.
//     The answer is `{ type: "keitora:block-state", state: BlockState | null }`,
//     where `null` means "no active session, or the service is unreachable".
//     Both the promise form and the callback form of `sendMessage` are faked,
//     so the page may use either.
//   - While the page is open, the service worker pushes the same message
//     (`{ type: "keitora:block-state", state }`) on every change, and the page
//     listens with `chrome.runtime.onMessage.addListener`.
//   - The page sends nothing else. It is an untrusted client: no message of
//     this contract can skip, stop, pause or unblock anything.
//
// `BlockState` below is that payload. Times are milliseconds: `remainingMs` is
// what was left of the current phase at `asOf`, and `asOf` is `Date.now()` on
// the service worker when it read the state, so that the page can discount the
// time the message spent in flight and keep counting on its own while the
// MV3 service worker sleeps.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { vi } from "vitest";

/** Message type of the state contract, for the request and for the pushes. */
export const BLOCK_STATE = "keitora:block-state";

/** The session state the block page shows. */
export interface BlockState {
  /** Phase that is blocking the site. */
  phase: "focus" | "short_break" | "long_break";
  /** 1-based number of the current cycle. */
  cycle: number;
  /** Total cycles of the session. */
  cycles: number;
  /** Milliseconds left in the current phase at `asOf`. */
  remainingMs: number;
  /** `Date.now()` on the service worker when it read the state. */
  asOf: number;
  /** Whether the session cannot be stopped early (strict mode). */
  strict: boolean;
  /** Epoch milliseconds when the strict lock ends, `null` when not strict. */
  strictEndsAt: number | null;
}

/** A call to `chrome.i18n.getMessage`. */
export interface I18nCall {
  key: string;
  substitutions: string[];
}

type MessageListener = (
  message: unknown,
  sender: unknown,
  sendResponse: (response?: unknown) => void,
) => unknown;

interface LastError {
  message: string;
}

interface RuntimeFake {
  id: string;
  lastError: LastError | undefined;
  getURL(path: string): string;
  sendMessage(
    message: unknown,
    callback?: (response?: unknown) => void,
  ): Promise<unknown> | undefined;
  onMessage: {
    addListener(listener: MessageListener): void;
    removeListener(listener: MessageListener): void;
    hasListener(listener: MessageListener): boolean;
  };
}

interface I18nFake {
  getMessage(key: string, substitutions?: string | string[]): string;
  getUILanguage(): string;
}

/** The URL whose host the page shows, unless a test asks for another one. */
export const BLOCKED_URL = "https://news.example.org/a/b?secret=1";

/** The host of {@link BLOCKED_URL}. */
export const BLOCKED_HOST = "news.example.org";

/** A fixed local "now" for the tests, so that formatted times are stable. */
export const NOW = new Date(2026, 0, 2, 13, 55, 28).getTime();

/** 14:32 left of a strict focus phase that ends at 14:10 local time. */
export function focusState(overrides: Partial<BlockState> = {}): BlockState {
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

/** The same session without the strict lock. */
export function relaxedState(overrides: Partial<BlockState> = {}): BlockState {
  return focusState({ strict: false, strictEndsAt: null, ...overrides });
}

/**
 * The `chrome` APIs the block page is given. Every answer is configured by the
 * test; nothing here knows how the page is built.
 */
export class ChromeFake {
  /** Every message the page sent, in order. */
  readonly sent: unknown[] = [];
  /** Every message key the page asked for, in order. */
  readonly i18nCalls: I18nCall[] = [];
  /** What `chrome.i18n.getUILanguage()` returns. */
  uiLanguage = "en";

  readonly api: { runtime: RuntimeFake; i18n: I18nFake };

  private listeners: MessageListener[] = [];
  private reply: () => unknown = () => ({ type: BLOCK_STATE, state: null });
  private failure: string | null = null;
  private throwing = false;

  constructor() {
    const runtime: RuntimeFake = {
      id: "keitoraacceptancetestsxxxxxxxxxx",
      lastError: undefined,
      getURL: (path: string) =>
        `chrome-extension://keitoraacceptancetestsxxxxxxxxxx/${path.replace(/^\//, "")}`,
      sendMessage: (message, callback) => this.send(message, callback),
      onMessage: {
        addListener: (listener) => {
          this.listeners.push(listener);
        },
        removeListener: (listener) => {
          this.listeners = this.listeners.filter((one) => one !== listener);
        },
        hasListener: (listener) => this.listeners.includes(listener),
      },
    };
    this.api = {
      runtime,
      i18n: {
        getMessage: (key, substitutions) => this.getMessage(key, substitutions),
        getUILanguage: () => this.uiLanguage,
      },
    };
  }

  /** Answers the page's request with this state (`null` = no session). */
  answerWith(state: BlockState | null): void {
    this.reply = () => ({ type: BLOCK_STATE, state });
    this.failure = null;
    this.throwing = false;
  }

  /** Answers with something that does not follow the contract. */
  answerRaw(response: unknown): void {
    this.reply = () => response;
    this.failure = null;
    this.throwing = false;
  }

  /** No receiving end: the promise rejects, the callback gets `lastError`. */
  failWith(message = "Could not establish connection."): void {
    this.failure = message;
    this.throwing = false;
  }

  /** `chrome.runtime.sendMessage` throws as soon as it is called. */
  throwOnSend(): void {
    this.throwing = true;
  }

  /** Pushes a new state to the page, as the service worker does. */
  push(state: BlockState | null): void {
    this.pushRaw({ type: BLOCK_STATE, state });
  }

  /** Pushes any message to the page's listeners. */
  pushRaw(message: unknown): void {
    for (const listener of [...this.listeners]) {
      listener(message, { id: this.api.runtime.id }, () => {});
    }
  }

  /** How many listeners the page registered. */
  get listenerCount(): number {
    return this.listeners.length;
  }

  /** The distinct message keys the page asked for, in order of first use. */
  get keys(): string[] {
    return [...new Set(this.i18nCalls.map((call) => call.key))];
  }

  /** Forgets the recorded `getMessage` calls, to measure the next render. */
  forgetI18nCalls(): void {
    this.i18nCalls.length = 0;
  }

  private getMessage(key: string, substitutions?: string | string[]): string {
    const subs =
      substitutions === undefined
        ? []
        : Array.isArray(substitutions)
          ? substitutions.map((one) => String(one))
          : [String(substitutions)];
    this.i18nCalls.push({ key, substitutions: subs });
    return subs.length === 0 ? `[[${key}]]` : `[[${key}:${subs.join("|")}]]`;
  }

  private send(
    message: unknown,
    callback?: (response?: unknown) => void,
  ): Promise<unknown> | undefined {
    this.sent.push(message);
    if (this.throwing) {
      throw new Error("chrome.runtime.sendMessage: extension context lost");
    }
    if (callback !== undefined) {
      void Promise.resolve().then(() => {
        if (this.failure !== null) {
          this.api.runtime.lastError = { message: this.failure };
          callback(undefined);
          this.api.runtime.lastError = undefined;
          return;
        }
        callback(this.reply());
      });
      return undefined;
    }
    if (this.failure !== null) {
      return Promise.reject(new Error(this.failure));
    }
    return Promise.resolve(this.reply());
  }
}

// A path, not `new URL("<relative>", import.meta.url)`: under jsdom, Vitest
// transforms modules in Vite's client environment, whose asset plugin rewrites
// that expression into a dev-server URL that `readFileSync` cannot open.
const PAGE_HTML = join(
  dirname(fileURLToPath(import.meta.url)),
  "../../../src/block-page/index.html",
);

/** The page's markup, as it ships. */
export function readPageHtml(): string {
  return readFileSync(PAGE_HTML, "utf-8");
}

/** Options of {@link loadBlockPage}. */
export interface LoadOptions {
  /** The blocked address; `null` leaves the `url` parameter out. */
  blockedUrl?: string | null;
  /** Put the address in the query as it is, without percent encoding. */
  raw?: boolean;
}

/**
 * Runs the block page: installs the fake `chrome`, puts the page's markup and
 * its URL in place, and loads its script, as a browser would.
 */
export async function loadBlockPage(
  chrome: ChromeFake,
  options: LoadOptions = {},
): Promise<void> {
  vi.stubGlobal("chrome", chrome.api);

  const blockedUrl =
    options.blockedUrl === undefined ? BLOCKED_URL : options.blockedUrl;
  const query =
    blockedUrl === null
      ? ""
      : `?url=${options.raw === true ? blockedUrl : encodeURIComponent(blockedUrl)}`;
  window.history.replaceState(null, "", `/block-page/index.html${query}`);

  const parsed = new DOMParser().parseFromString(readPageHtml(), "text/html");
  document.replaceChild(
    document.importNode(parsed.documentElement, true),
    document.documentElement,
  );

  vi.resetModules();
  await import("../../../src/block-page/block-page");
  document.dispatchEvent(new Event("DOMContentLoaded", { bubbles: true }));
  window.dispatchEvent(new Event("load"));
  await flush();
}

/** Lets pending work run, and moves the clock on when it is faked. */
export async function flush(ms = 0): Promise<void> {
  if (vi.isFakeTimers()) {
    await vi.advanceTimersByTimeAsync(ms);
    return;
  }
  await new Promise((resolve) => setTimeout(resolve, ms));
}

/** The page's visible text, with runs of whitespace collapsed. */
export function pageText(): string {
  return (document.body.textContent ?? "").replace(/\s+/g, " ").trim();
}

/** Anything a user could activate to skip, unblock or leave the block. */
export const CONTROL_SELECTOR = [
  "button",
  "input",
  "select",
  "textarea",
  "form",
  "a[href]",
  '[role="button"]',
  '[role="link"]',
  '[role="menuitem"]',
  '[role="switch"]',
  '[role="checkbox"]',
  "[contenteditable]",
  "[onclick]",
].join(", ");

/** The controls the page has, which in this page must always be none. */
export function controls(): Element[] {
  return [...document.querySelectorAll(CONTROL_SELECTOR)];
}

/** The sorted tag names of every element in the page. */
export function elementCensus(): string[] {
  return [...document.querySelectorAll("*")]
    .map((element) => element.tagName.toLowerCase())
    .sort();
}

/** The `[[key]]` and `[[key:a|b]]` stand-ins left in a piece of text. */
export function sentinels(text: string): string[] {
  return [...text.matchAll(/\[\[(.*?)\]\]/g)].map((match) => match[1] ?? "");
}

/** Text with the message stand-ins removed, to look for untranslated words. */
export function withoutSentinels(text: string): string {
  return text.replace(/\[\[.*?\]\]/g, " ");
}

/**
 * Records every way of turning a string into markup, so that a test can prove
 * the page never uses one (a finding carried over from the T-2.3 review:
 * nothing from outside is ever written as HTML). Clearing a container with an
 * empty string is not recorded.
 */
export class MarkupGuard {
  readonly violations: string[] = [];

  private restore: (() => void)[] = [];

  install(): void {
    this.patchProperty(Element.prototype, "innerHTML");
    this.patchProperty(Element.prototype, "outerHTML");
    this.patchMethod(Element.prototype, "insertAdjacentHTML");
    this.patchMethod(Document.prototype, "write");
    this.patchMethod(Document.prototype, "writeln");
    this.patchMethod(Range.prototype, "createContextualFragment");
  }

  uninstall(): void {
    for (const undo of this.restore.reverse()) {
      undo();
    }
    this.restore = [];
    this.violations.length = 0;
  }

  private patchProperty(target: object, name: string): void {
    const descriptor = Object.getOwnPropertyDescriptor(target, name);
    if (descriptor?.set === undefined) {
      return;
    }
    const original = descriptor.set;
    const violations = this.violations;
    Object.defineProperty(target, name, {
      ...descriptor,
      set(this: unknown, value: unknown) {
        if (String(value) !== "") {
          violations.push(`${name} = ${String(value)}`);
        }
        original.call(this, value);
      },
    });
    this.restore.push(() => {
      Object.defineProperty(target, name, descriptor);
    });
  }

  private patchMethod(target: object, name: string): void {
    const original = Reflect.get(target, name) as unknown;
    if (typeof original !== "function") {
      return;
    }
    const violations = this.violations;
    const call = original as (this: unknown, ...args: unknown[]) => unknown;
    Reflect.set(target, name, function (this: unknown, ...args: unknown[]) {
      violations.push(`${name}(${args.map((one) => String(one)).join(", ")})`);
      return call.apply(this, args);
    });
    this.restore.push(() => {
      Reflect.set(target, name, original);
    });
  }
}
