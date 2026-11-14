// SPDX-License-Identifier: MIT OR Apache-2.0

// The page a blocked tab lands on. It names the site, shows what is left of
// the current phase and, in a strict session, says until when the block cannot
// be lifted.
//
// The page is an untrusted client (AGENTS.md): it only asks the service worker
// for the session's state and shows it. It offers no way to skip, pause, stop
// or unblock anything, and it never turns data from outside into markup: every
// string it shows is written with `textContent`.
//
// The page cannot reach the service itself. The service worker, which T-4.2
// implements, answers `{ type: "keitora:block-state", state }` to the request
// below and pushes the same message whenever the state changes.

import {
  BLOCK_STATE,
  type BlockState,
  type Phase,
  blockedHost,
  formatClock,
  formatTimeOfDay,
  isFromServiceWorker,
  isStateMessage,
  parseState,
  remainingAt,
} from "./block-state";

/** How often the page redraws the clock while it is open. */
const TICK_MS = 1000;

/** The message that names each phase. */
const PHASE_KEYS: Record<Phase, string> = {
  focus: "blockPagePhaseFocus",
  short_break: "blockPagePhaseShortBreak",
  long_break: "blockPagePhaseLongBreak",
};

/** The `chrome` APIs the page uses, which is as little as it needs. */
interface ChromeApi {
  runtime: {
    /** The extension's own ID, which its service worker's messages carry. */
    id: string;
    sendMessage(message: unknown): Promise<unknown>;
    onMessage: {
      addListener(listener: (message: unknown, sender: unknown) => void): void;
    };
  };
  i18n: {
    getMessage(key: string, substitutions: string[]): string;
    getUILanguage(): string;
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * The `chrome` APIs, or `null` when the page runs outside an extension. It can
 * show nothing useful then, because even its own text comes from `chrome.i18n`.
 */
function chromeApi(): ChromeApi | null {
  const candidate = (globalThis as { chrome?: unknown }).chrome;
  if (!isRecord(candidate)) {
    return null;
  }
  const { runtime, i18n } = candidate;
  if (!isRecord(runtime) || !isRecord(i18n)) {
    return null;
  }
  const { onMessage } = runtime;
  if (
    typeof runtime.id !== "string" ||
    typeof runtime.sendMessage !== "function" ||
    !isRecord(onMessage) ||
    typeof onMessage.addListener !== "function" ||
    typeof i18n.getMessage !== "function" ||
    typeof i18n.getUILanguage !== "function"
  ) {
    return null;
  }
  // The shape was just checked member by member. The cast is needed because
  // the extension declares no ambient browser types, so `chrome` is `unknown`.
  return candidate as unknown as ChromeApi;
}

/** Writes a slot's text. Data from outside arrives as text, never as markup. */
function fill(id: string, text: string): void {
  const slot = document.getElementById(id);
  if (slot !== null) {
    slot.textContent = text;
  }
}

/** Shows or hides a part of the page. */
function show(id: string, visible: boolean): void {
  document.getElementById(id)?.toggleAttribute("hidden", !visible);
}

/** The page, which redraws itself whenever the state or the clock changes. */
class BlockPage {
  private state: BlockState | null = null;
  private answered = false;

  constructor(
    private readonly api: ChromeApi,
    private readonly host: string | null,
    private readonly language: string,
  ) {}

  /** Puts the page in place and starts following the session's state. */
  start(): void {
    document.documentElement.lang = this.language;
    document.title = this.message("blockPageTitle");

    // Chrome delivers a message from any page of the extension to every other
    // one, so the block page hears the requests of other block pages, which a
    // hostile site can open in a frame at will. Only a state from the service
    // worker counts; anything else changes nothing, not even `answered`.
    this.api.runtime.onMessage.addListener(
      (message: unknown, sender: unknown) => {
        if (
          !isFromServiceWorker(sender, this.api.runtime.id) ||
          !isStateMessage(message)
        ) {
          return;
        }
        this.answered = true;
        this.state = parseState(message.state);
        this.render();
      },
    );

    // Nothing is drawn until the first answer arrives: an empty page for a
    // moment is better than saying there is no session while the answer is
    // still on its way.
    void this.ask();

    window.setInterval(() => {
      this.render();
    }, TICK_MS);
  }

  /**
   * Asks the service worker for the state, once, when the page opens. The
   * page then counts down on its own and waits for what the worker pushes; it
   * never asks for anything else, and asks for nothing that changes a session.
   */
  private async ask(): Promise<void> {
    const answer = await this.request();
    // A state pushed while the answer was on its way is the newer one.
    if (!this.answered) {
      this.answered = true;
      this.state = isStateMessage(answer) ? parseState(answer.state) : null;
    }
    this.render();
  }

  /** The service worker's answer, or `null` when it does not come. */
  private async request(): Promise<unknown> {
    try {
      return await this.api.runtime.sendMessage({ type: BLOCK_STATE });
    } catch {
      // No receiving end, or the extension is gone: the page says so instead.
      return null;
    }
  }

  /** Draws every slot from the current state. */
  private render(): void {
    const state = this.state;

    fill(
      "heading",
      this.host === null
        ? this.message("blockPageHeadingUnknownSite")
        : this.message("blockPageHeading", [this.host]),
    );
    fill("footer-text", this.message("blockPageFooter"));

    show("session", state !== null);
    show("explanation", state !== null);
    show("no-data", state === null);
    fill("no-data", state === null ? this.message("blockPageNoData") : "");
    fill(
      "explanation",
      state === null ? "" : this.message("blockPageExplanation"),
    );

    if (state === null) {
      // Hidden text is still read by a screen reader and still found by a
      // search, so an unknown state leaves no stale text behind.
      fill("clock", "");
      fill("remaining-label", "");
      fill("phase", "");
      fill("cycle", "");
      this.fillStrictNotice("");
      return;
    }

    fill("clock", formatClock(remainingAt(state, Date.now())));
    fill("remaining-label", this.message("blockPageRemaining"));
    fill("phase", this.message(PHASE_KEYS[state.phase]));
    fill(
      "cycle",
      this.message("blockPageCycle", [
        String(state.cycle),
        String(state.cycles),
      ]),
    );
    this.fillStrictNotice(this.strictNotice(state));
  }

  private fillStrictNotice(text: string): void {
    fill("strict-text", text);
    show("strict-notice", text !== "");
  }

  /** What the page says about the strict lock, or `""` when there is none. */
  private strictNotice(state: BlockState): string {
    if (!state.strict) {
      return "";
    }
    const endsAt = formatTimeOfDay(state.strictEndsAt, this.language);
    return endsAt === ""
      ? this.message("blockPageStrictLocked")
      : this.message("blockPageStrictUntil", [endsAt]);
  }

  private message(key: string, substitutions: string[] = []): string {
    return this.api.i18n.getMessage(key, substitutions);
  }
}

function start(): void {
  const api = chromeApi();
  if (api === null) {
    return;
  }
  new BlockPage(
    api,
    blockedHost(window.location.search),
    api.i18n.getUILanguage(),
  ).start();
}

// A module script runs after the document is parsed, so the page is normally
// ready here; the check keeps it correct if it is ever loaded another way.
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", start, { once: true });
} else {
  start();
}
