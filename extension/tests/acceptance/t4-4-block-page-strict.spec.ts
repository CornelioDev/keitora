// SPDX-License-Identifier: MIT OR Apache-2.0
// @vitest-environment jsdom

// Acceptance tests for T-4.4: strict mode on the block page, and the page's
// accessibility. See t4-4-block-page.spec.ts for where the criteria come from.
//
//  - AC-5 the page offers no control that skips, unblocks or stops the
//    session: such a control is absent from the DOM, not hidden or disabled,
//    because the extension is untrusted (AGENTS.md). Decision D1 of the QA
//    report: the page has no control in any mode, as in the mockup; the
//    session is controlled from the desktop app only.
//      offers_no_control_in_strict_mode
//      offers_no_control_outside_strict_mode
//      has_no_control_after_the_service_worker_pushes_a_new_state
//      has_no_element_that_links_back_to_the_blocked_site
//  - AC-6 in strict mode the page explains that the block cannot be lifted
//    until the session's end time, and shows that time; outside strict mode it
//    claims nothing of the sort
//      explains_that_a_strict_session_cannot_be_unblocked_until_its_end_time
//      shows_no_strict_notice_outside_strict_mode
//      shows_no_broken_time_when_a_strict_session_has_no_end_time
//      stops_showing_the_strict_notice_when_the_session_stops_being_strict
//  - AC-12 accessibility (WCAG 2.2 AA, AGENTS.md): one main heading, images
//    labelled or hidden, no focus trap through a positive tab order
//      has_exactly_one_main_heading
//      hides_or_labels_every_image
//      sets_no_positive_tab_order

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BLOCKED_HOST,
  ChromeFake,
  NOW,
  controls,
  flush,
  focusState,
  loadBlockPage,
  pageText,
  relaxedState,
  withoutSentinels,
} from "./support/t4-4-block-page";

/** 14:10 local time, the end of the strict session in the fixtures. */
const STRICT_END = /\b(14[:.]10|0?2[:.]10)\b/;

let chrome: ChromeFake;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  chrome = new ChromeFake();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("no way out of the block", () => {
  it("offers_no_control_in_strict_mode", async () => {
    chrome.answerWith(focusState({ strict: true }));

    await loadBlockPage(chrome);

    expect(controls().map((element) => element.outerHTML)).toEqual([]);
  });

  it("offers_no_control_outside_strict_mode", async () => {
    chrome.answerWith(relaxedState());

    await loadBlockPage(chrome);

    expect(controls().map((element) => element.outerHTML)).toEqual([]);
  });

  it("has_no_control_after_the_service_worker_pushes_a_new_state", async () => {
    chrome.answerWith(focusState());
    await loadBlockPage(chrome);

    chrome.push(relaxedState({ phase: "short_break" }));
    await flush();
    chrome.push(focusState());
    await flush();
    chrome.push(null);
    await flush();

    expect(controls().map((element) => element.outerHTML)).toEqual([]);
  });

  it("has_no_element_that_links_back_to_the_blocked_site", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    const html = document.documentElement.outerHTML;
    expect(html).not.toContain(BLOCKED_HOST + "/");
    expect(html).not.toMatch(/href\s*=\s*["']https?:/i);
  });
});

describe("the strict notice", () => {
  it("explains_that_a_strict_session_cannot_be_unblocked_until_its_end_time", async () => {
    chrome.answerWith(focusState({ strict: true }));

    await loadBlockPage(chrome);
    const strictKeys = chrome.keys;

    expect(pageText()).toMatch(STRICT_END);
    chrome.forgetI18nCalls();
    chrome.answerWith(relaxedState());
    await loadBlockPage(chrome);
    // At least one message is used only when the session is strict.
    expect(strictKeys.some((key) => !chrome.keys.includes(key))).toBe(true);
  });

  it("shows_no_strict_notice_outside_strict_mode", async () => {
    chrome.answerWith(relaxedState());

    await loadBlockPage(chrome);

    expect(pageText()).not.toMatch(STRICT_END);
  });

  it("shows_no_broken_time_when_a_strict_session_has_no_end_time", async () => {
    chrome.answerWith(focusState({ strict: true, strictEndsAt: null }));

    await loadBlockPage(chrome);

    const text = pageText();
    expect(text).toContain(BLOCKED_HOST);
    // Message keys are left out: only what the page wrote itself is checked.
    expect(withoutSentinels(text)).not.toMatch(/NaN|Invalid|null|undefined/);
  });

  it("stops_showing_the_strict_notice_when_the_session_stops_being_strict", async () => {
    chrome.answerWith(focusState({ strict: true }));
    await loadBlockPage(chrome);
    expect(pageText()).toMatch(STRICT_END);

    chrome.push(relaxedState());
    await flush();

    expect(pageText()).not.toMatch(STRICT_END);
  });
});

describe("accessibility", () => {
  it("has_exactly_one_main_heading", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(document.querySelectorAll("h1")).toHaveLength(1);
  });

  it("hides_or_labels_every_image", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    for (const image of document.querySelectorAll("img, svg, [role='img']")) {
      const hidden = image.getAttribute("aria-hidden") === "true";
      const name =
        image.getAttribute("aria-label") ??
        image.getAttribute("alt") ??
        image.querySelector("title")?.textContent ??
        "";
      expect(hidden || name.trim() !== "", image.outerHTML).toBe(true);
    }
  });

  it("sets_no_positive_tab_order", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    for (const element of document.querySelectorAll("[tabindex]")) {
      expect(Number(element.getAttribute("tabindex"))).toBeLessThanOrEqual(0);
    }
  });
});
