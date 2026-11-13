// SPDX-License-Identifier: MIT OR Apache-2.0
// @vitest-environment jsdom

// Acceptance tests for T-4.4, "Block page: site, remaining time; no skip
// control in strict mode; localized" (docs/roadmap.md), against the block page
// of docs/ui-stack.md (line 36: plain HTML and CSS in the extension, with the
// same design tokens; the visual reference is
// docs/assets/mockups/08-block-page.png).
//
// The criteria are derived from the roadmap line, that mockup and the rules of
// AGENTS.md (the extension is untrusted, no hard-coded UI strings, WCAG 2.2
// AA). The contract with the service worker, which T-4.2 implements, is
// described in support/t4-4-block-page.ts.
//
//  - AC-1 the page names the blocked site, taken from the `url` parameter of
//    its own address, as the host only
//      shows_the_host_of_the_blocked_address
//      accepts_a_blocked_address_that_is_not_percent_encoded
//      shows_nothing_of_the_blocked_address_but_its_host
//  - AC-2 the page shows the time left in the current phase, as minutes and
//    seconds, counting down while it is open
//      shows_the_remaining_time_as_minutes_and_seconds
//      pads_minutes_and_seconds_to_two_digits
//      shows_a_phase_longer_than_an_hour_without_losing_minutes
//      discounts_the_time_elapsed_since_the_service_worker_read_the_state
//      counts_down_while_the_page_stays_open
//  - AC-3 the page shows the phase and the cycle
//      shows_the_cycle_and_the_total_of_cycles
//      uses_a_different_message_for_each_phase
//  - AC-4 the page reads the state through the `keitora:block-state` contract
//    and sends nothing that could change the session
//      asks_the_service_worker_for_the_state_when_it_opens
//      listens_for_the_states_the_service_worker_pushes
//      shows_a_state_pushed_by_the_service_worker
//      sends_no_message_that_could_change_the_session
//      makes_no_network_request
//  - AC-7 missing, late or impossible data degrades safely
//      falls_back_when_the_service_worker_does_not_answer
//      falls_back_when_sending_the_request_throws
//      falls_back_when_there_is_no_active_session
//      falls_back_when_the_answer_does_not_follow_the_contract
//      keeps_counting_down_when_the_service_worker_stops_answering
//      never_shows_a_negative_time_when_the_state_is_stale
//      survives_a_state_with_impossible_values

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BLOCKED_HOST,
  BLOCK_STATE,
  ChromeFake,
  NOW,
  flush,
  focusState,
  loadBlockPage,
  pageText,
  withoutSentinels,
} from "./support/t4-4-block-page";

/** A clock reading as minutes and seconds, or hours, minutes and seconds. */
const CLOCK = /\d{1,3}:\d{2}(:\d{2})?/;

let chrome: ChromeFake;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  chrome = new ChromeFake();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the blocked site", () => {
  it("shows_the_host_of_the_blocked_address", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(pageText()).toContain(BLOCKED_HOST);
    const heading = document.querySelector("h1");
    expect(heading?.textContent ?? "").toContain(BLOCKED_HOST);
  });

  it("accepts_a_blocked_address_that_is_not_percent_encoded", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome, {
      blockedUrl: "https://videos.example.net/watch?v=abc",
      raw: true,
    });

    expect(pageText()).toContain("videos.example.net");
  });

  it("shows_nothing_of_the_blocked_address_but_its_host", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    const text = pageText();
    expect(text).not.toContain("/a/b");
    expect(text).not.toContain("secret");
    expect(text).not.toContain("https://");
  });
});

describe("the remaining time", () => {
  it("shows_the_remaining_time_as_minutes_and_seconds", async () => {
    chrome.answerWith(focusState({ remainingMs: 872_000 }));

    await loadBlockPage(chrome);

    expect(pageText()).toMatch(/\b14:32\b/);
  });

  it("pads_minutes_and_seconds_to_two_digits", async () => {
    chrome.answerWith(focusState({ remainingMs: 65_000 }));

    await loadBlockPage(chrome);

    expect(pageText()).toMatch(/\b01:05\b/);
  });

  it("shows_a_phase_longer_than_an_hour_without_losing_minutes", async () => {
    // The engine allows a focus phase of up to 120 minutes (T-1.3).
    chrome.answerWith(focusState({ remainingMs: 7_200_000 }));

    await loadBlockPage(chrome);

    expect(pageText()).toMatch(/\b(120:00|0?2:00:00)\b/);
  });

  it("discounts_the_time_elapsed_since_the_service_worker_read_the_state", async () => {
    chrome.answerWith(focusState({ remainingMs: 872_000, asOf: NOW - 30_000 }));

    await loadBlockPage(chrome);

    expect(pageText()).toMatch(/\b14:02\b/);
  });

  it("counts_down_while_the_page_stays_open", async () => {
    // An MV3 service worker sleeps, so the page cannot rely on being pushed a
    // new state every second.
    chrome.answerWith(focusState({ remainingMs: 872_000 }));
    await loadBlockPage(chrome);
    expect(pageText()).toMatch(/\b14:32\b/);

    await flush(30_000);

    expect(pageText()).toMatch(/\b14:02\b/);
    expect(pageText()).not.toMatch(/\b14:32\b/);
  });
});

describe("the phase and the cycle", () => {
  it("shows_the_cycle_and_the_total_of_cycles", async () => {
    chrome.answerWith(focusState({ cycle: 2, cycles: 4 }));

    await loadBlockPage(chrome);

    const carriesBoth = chrome.i18nCalls.some(
      (call) =>
        call.substitutions.includes("2") && call.substitutions.includes("4"),
    );
    expect(carriesBoth, JSON.stringify(chrome.i18nCalls)).toBe(true);
  });

  it("uses_a_different_message_for_each_phase", async () => {
    chrome.answerWith(focusState());
    await loadBlockPage(chrome);
    const focusKeys = chrome.keys;

    chrome.forgetI18nCalls();
    chrome.answerWith(focusState({ phase: "short_break" }));
    await loadBlockPage(chrome);
    const shortBreakKeys = chrome.keys;

    chrome.forgetI18nCalls();
    chrome.answerWith(focusState({ phase: "long_break" }));
    await loadBlockPage(chrome);
    const longBreakKeys = chrome.keys;

    expect(focusKeys).not.toEqual(shortBreakKeys);
    expect(shortBreakKeys).not.toEqual(longBreakKeys);
    expect(focusKeys).not.toEqual(longBreakKeys);
  });
});

describe("the contract with the service worker", () => {
  it("asks_the_service_worker_for_the_state_when_it_opens", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(chrome.sent[0]).toEqual({ type: BLOCK_STATE });
  });

  it("listens_for_the_states_the_service_worker_pushes", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(chrome.listenerCount).toBeGreaterThan(0);
  });

  it("shows_a_state_pushed_by_the_service_worker", async () => {
    chrome.answerWith(focusState({ remainingMs: 872_000 }));
    await loadBlockPage(chrome);

    chrome.push(
      focusState({ phase: "short_break", remainingMs: 60_000, asOf: NOW }),
    );
    await flush();

    expect(pageText()).toMatch(/\b01:00\b/);
    expect(pageText()).not.toMatch(/\b14:32\b/);
  });

  it("sends_no_message_that_could_change_the_session", async () => {
    chrome.answerWith(focusState());
    await loadBlockPage(chrome);

    chrome.push(focusState({ strict: false, strictEndsAt: null }));
    await flush(60_000);

    for (const message of chrome.sent) {
      expect(message).toEqual({ type: BLOCK_STATE });
    }
    expect(JSON.stringify(chrome.sent)).not.toMatch(
      /skip|stop|pause|resume|unblock|unlock|start|allow|disable/i,
    );
  });

  it("makes_no_network_request", async () => {
    const fetchSpy = vi.fn();
    const beaconSpy = vi.fn();
    const socketSpy = vi.fn();
    const requestSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    // Replacing the whole `navigator` would take jsdom's own properties away.
    Object.defineProperty(navigator, "sendBeacon", {
      value: beaconSpy,
      configurable: true,
    });
    vi.stubGlobal("WebSocket", socketSpy);
    vi.stubGlobal("XMLHttpRequest", requestSpy);
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);
    await flush(60_000);

    expect(fetchSpy).not.toHaveBeenCalled();
    expect(beaconSpy).not.toHaveBeenCalled();
    expect(socketSpy).not.toHaveBeenCalled();
    expect(requestSpy).not.toHaveBeenCalled();
  });
});

describe("missing or late data", () => {
  it("falls_back_when_the_service_worker_does_not_answer", async () => {
    chrome.answerWith(focusState());
    await loadBlockPage(chrome);
    const keysWithData = chrome.keys;

    chrome.forgetI18nCalls();
    chrome.failWith();
    await loadBlockPage(chrome);

    const text = pageText();
    expect(text).toContain(BLOCKED_HOST);
    expect(text).not.toMatch(CLOCK);
    // The page says so instead of showing a timer that is not there.
    expect(chrome.keys.some((key) => !keysWithData.includes(key))).toBe(true);
  });

  it("falls_back_when_sending_the_request_throws", async () => {
    chrome.throwOnSend();

    await loadBlockPage(chrome);

    expect(pageText()).toContain(BLOCKED_HOST);
    expect(pageText()).not.toMatch(CLOCK);
  });

  it("falls_back_when_there_is_no_active_session", async () => {
    chrome.answerWith(null);

    await loadBlockPage(chrome);

    expect(pageText()).toContain(BLOCKED_HOST);
    expect(pageText()).not.toMatch(CLOCK);
  });

  it("falls_back_when_the_answer_does_not_follow_the_contract", async () => {
    chrome.answerRaw({ type: BLOCK_STATE, state: "not a state" });

    await loadBlockPage(chrome);

    const text = pageText();
    expect(text).toContain(BLOCKED_HOST);
    expect(text).not.toMatch(CLOCK);
    // Message keys are left out: only what the page wrote itself is checked.
    expect(withoutSentinels(text)).not.toMatch(/NaN|undefined|null|Invalid/);
  });

  it("keeps_counting_down_when_the_service_worker_stops_answering", async () => {
    chrome.answerWith(focusState({ remainingMs: 872_000 }));
    await loadBlockPage(chrome);

    chrome.failWith();
    await flush(30_000);

    expect(pageText()).toMatch(/\b14:02\b/);
  });

  it("never_shows_a_negative_time_when_the_state_is_stale", async () => {
    chrome.answerWith(
      focusState({ remainingMs: 872_000, asOf: NOW - 1_200_000 }),
    );

    await loadBlockPage(chrome);
    await flush(60_000);

    const text = pageText();
    expect(text).toMatch(/\b0?0:00\b/);
    expect(withoutSentinels(text)).not.toMatch(/-\d/);
    expect(withoutSentinels(text)).not.toMatch(/NaN/);
  });

  it("survives_a_state_with_impossible_values", async () => {
    chrome.answerRaw({
      type: BLOCK_STATE,
      state: {
        phase: "not-a-phase",
        cycle: 0,
        cycles: 0,
        remainingMs: -5_000,
        asOf: Number.NaN,
        strict: true,
        strictEndsAt: Number.NaN,
      },
    });

    await loadBlockPage(chrome);
    await flush(5_000);

    const text = pageText();
    expect(text).toContain(BLOCKED_HOST);
    expect(withoutSentinels(text)).not.toMatch(/NaN|Invalid|-\d/);
  });
});
