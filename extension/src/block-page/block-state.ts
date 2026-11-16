// SPDX-License-Identifier: MIT OR Apache-2.0

// The block page's reasoning, with no browser in it: what the page may
// believe, and how it turns that into the strings it shows. Everything here
// comes from outside the page (the address of the site that was blocked, a
// state from a service worker the page cannot verify), so every function here
// treats its input as untrusted.

/** Message type of the state contract, for the request and for the pushes. */
export const BLOCK_STATE = "keitora:block-state";

/** Phase of the session that is blocking the site. */
export type Phase = "focus" | "short_break" | "long_break";

/** The session state the block page shows. */
export interface BlockState {
  /** Phase that is blocking the site. */
  phase: Phase;
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

/**
 * The most cycles a state may claim. It is a sanity bound, well above what the
 * engine allows (keitora-core's `MAX_CYCLES`, 8), so that the page never shows
 * an absurd number, yet does not go blank if the engine allows a few more.
 */
export const MAX_CYCLES = 99;

/**
 * The longest a phase may claim to have left: a day. The engine's longest
 * phase is 120 minutes (`MAX_FOCUS_MIN`); anything beyond a day is a broken
 * state, not a long phase.
 */
export const MAX_REMAINING_MS = 24 * 60 * 60 * 1000;

/** The longest host name DNS allows, which the URL parser does not enforce. */
export const MAX_HOST_LENGTH = 253;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isPhase(value: unknown): value is Phase {
  return value === "focus" || value === "short_break" || value === "long_break";
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isCount(value: unknown): value is number {
  return (
    isFiniteNumber(value) &&
    Number.isInteger(value) &&
    value >= 1 &&
    value <= MAX_CYCLES
  );
}

/**
 * Whether a message, or an answer to the request, carries a state. The page's
 * own request, `{ type: "keitora:block-state" }`, has the same type but no
 * `state`; Chrome delivers it to every other page of the extension too, so it
 * must never count as a state. A real answer or push always has `state`, even
 * when it is `null`.
 */
export function isStateMessage(
  message: unknown,
): message is { state: unknown } {
  return (
    isRecord(message) &&
    message.type === BLOCK_STATE &&
    Object.hasOwn(message, "state")
  );
}

/**
 * Whether a message comes from the extension's service worker, the only
 * sender whose state the page believes. `extensionId` is
 * `chrome.runtime.id`. A sender with a `tab` is a page of the extension open
 * in a tab or in a frame, such as another block page, possibly framed by a
 * hostile site; a sender with another `id` is another extension.
 */
export function isFromServiceWorker(
  sender: unknown,
  extensionId: string,
): boolean {
  return (
    isRecord(sender) && sender.id === extensionId && sender.tab === undefined
  );
}

/**
 * Reads a state, field by field, from a value the page did not produce.
 * Returns `null` for anything unexpected, including the `null` that means "no
 * active session": the page would rather say nothing than show a time it
 * cannot stand behind. Values of the right type but out of range, such as
 * cycle 9 of 4 or a phase with years left, are unexpected too. A slightly
 * negative remaining time is not: a state read as the phase ends reads as 0.
 */
export function parseState(value: unknown): BlockState | null {
  if (!isRecord(value)) {
    return null;
  }
  const { phase, cycle, cycles, remainingMs, asOf, strict, strictEndsAt } =
    value;
  if (!isPhase(phase) || !isCount(cycle) || !isCount(cycles)) {
    return null;
  }
  if (cycle > cycles) {
    return null;
  }
  if (!isFiniteNumber(remainingMs) || !isFiniteNumber(asOf)) {
    return null;
  }
  if (remainingMs > MAX_REMAINING_MS) {
    return null;
  }
  if (typeof strict !== "boolean") {
    return null;
  }
  if (strictEndsAt !== null && !isFiniteNumber(strictEndsAt)) {
    return null;
  }
  return {
    phase,
    cycle,
    cycles,
    remainingMs: Math.max(0, remainingMs),
    asOf,
    strict,
    strictEndsAt,
  };
}

/**
 * The host of the blocked address, which reaches the page as the `url`
 * parameter of its own address (percent encoded or not), or `null` when there
 * is none to show. Only `http` and `https` name a site: keitora-core blocks no
 * other scheme, and an address the page cannot parse says nothing about who
 * was blocked. Nothing but the host is ever shown, so that a blocked page
 * cannot have the block page repeat a path it chose, and a host longer than
 * DNS allows names no site, so that a crafted address cannot push the rest of
 * the page off the screen.
 *
 * @param search the query of the page's own address, `location.search`.
 */
export function blockedHost(search: string): string | null {
  const raw = new URLSearchParams(search).get("url");
  if (raw === null) {
    return null;
  }
  let address: URL;
  try {
    address = new URL(raw);
  } catch {
    return null;
  }
  if (address.protocol !== "http:" && address.protocol !== "https:") {
    return null;
  }
  if (address.hostname === "" || address.hostname.length > MAX_HOST_LENGTH) {
    return null;
  }
  return address.host;
}

/**
 * What is left of the phase at `now`, never less than zero. The service worker
 * read the state at `asOf` and an MV3 service worker sleeps, so the page
 * discounts the time since then and keeps counting on its own.
 */
export function remainingAt(state: BlockState, now: number): number {
  const elapsed = Number.isFinite(now) ? now - state.asOf : 0;
  return Math.max(0, state.remainingMs - Math.max(0, elapsed));
}

/**
 * A countdown as `mm:ss`, both padded to two digits. Minutes are not wrapped
 * into hours: a phase of two hours reads `120:00`.
 */
export function formatClock(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  return `${pad(Math.floor(seconds / 60))}:${pad(seconds % 60)}`;
}

function pad(value: number): string {
  return value < 10 ? `0${String(value)}` : String(value);
}

/**
 * A time of day written as `language` writes it, or `""` when there is none
 * to write. The strict notice leaves the time out rather than showing a
 * broken one.
 */
export function formatTimeOfDay(
  epochMs: number | null,
  language: string,
): string {
  if (epochMs === null || !Number.isFinite(epochMs)) {
    return "";
  }
  try {
    return new Intl.DateTimeFormat(language, {
      hour: "numeric",
      minute: "2-digit",
    }).format(new Date(epochMs));
  } catch {
    // An interface language Intl does not know: better no time than a crash.
    return "";
  }
}
