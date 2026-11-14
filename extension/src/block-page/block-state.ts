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
  return isFiniteNumber(value) && Number.isInteger(value) && value > 0;
}

/** Whether a message, or an answer to the request, follows the contract. */
export function isStateMessage(
  message: unknown,
): message is { state: unknown } {
  return isRecord(message) && message.type === BLOCK_STATE;
}

/**
 * Reads a state, field by field, from a value the page did not produce.
 * Returns `null` for anything unexpected, including the `null` that means "no
 * active session": the page would rather say nothing than show a time it
 * cannot stand behind.
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
  if (!isFiniteNumber(remainingMs) || !isFiniteNumber(asOf)) {
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
 * cannot have the block page repeat a path it chose.
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
  return address.host === "" ? null : address.host;
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
