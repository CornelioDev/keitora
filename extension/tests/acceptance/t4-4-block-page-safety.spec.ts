// SPDX-License-Identifier: MIT OR Apache-2.0
// @vitest-environment jsdom

// Acceptance tests for T-4.4: everything the page shows comes from outside
// (the blocked address is chosen by the site that was blocked, the state comes
// from a service worker the page cannot verify), so none of it may ever become
// markup. This pins the finding carried over from the T-2.3 review: no
// `innerHTML`. See t4-4-block-page.spec.ts for where the criteria come from.
//
//  - AC-9 outside data is rendered as text, never as markup
//      never_turns_a_string_into_markup
//      a_blocked_address_that_is_markup_adds_no_element
//      a_blocked_address_that_is_markup_runs_no_script
//      markup_in_the_path_of_the_blocked_address_is_not_rendered
//      a_pushed_state_full_of_markup_adds_no_element
//      the_page_has_no_inline_event_handler
//  - AC-1 only `http` and `https` addresses name a site (keitora-core blocks
//    no other scheme, T-1.4)
//      an_address_that_is_not_http_names_no_site
//      a_missing_address_names_no_site

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BLOCK_STATE,
  ChromeFake,
  MarkupGuard,
  NOW,
  flush,
  focusState,
  loadBlockPage,
  pageText,
  withoutSentinels,
} from "./support/t4-4-block-page";

const XSS_FLAG = "__keitoraBlockPageXss";
const PAYLOAD = `<img src=x onerror="globalThis.${XSS_FLAG} = true">`;

/** Elements that would mean injected markup, plus the page's own script tag. */
function risky(): number {
  return document.querySelectorAll("script, iframe, object, embed, form")
    .length;
}

let chrome: ChromeFake;
let guard: MarkupGuard;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  chrome = new ChromeFake();
  guard = new MarkupGuard();
  guard.install();
});

afterEach(() => {
  guard.uninstall();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(globalThis, XSS_FLAG);
});

describe("nothing becomes markup", () => {
  it("never_turns_a_string_into_markup", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);
    chrome.push(focusState({ phase: "short_break", remainingMs: 60_000 }));
    await flush(1_000);

    expect(guard.violations).toEqual([]);
  });

  it("a_blocked_address_that_is_markup_adds_no_element", async () => {
    chrome.answerWith(focusState());
    await loadBlockPage(chrome, { blockedUrl: null });
    const before = risky();

    await loadBlockPage(chrome, { blockedUrl: PAYLOAD });

    expect(risky()).toBe(before);
    expect(document.querySelector("[onerror]")).toBeNull();
    expect(guard.violations).toEqual([]);
  });

  it("a_blocked_address_that_is_markup_runs_no_script", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome, { blockedUrl: PAYLOAD, raw: true });
    await flush(1_000);

    expect(Reflect.get(globalThis, XSS_FLAG)).toBeUndefined();
    // If it is shown at all, it is shown as the text it is.
    const shown = pageText();
    expect(shown.includes(PAYLOAD) || !shown.includes("img src")).toBe(true);
  });

  it("markup_in_the_path_of_the_blocked_address_is_not_rendered", async () => {
    chrome.answerWith(focusState());
    const before = risky();

    await loadBlockPage(chrome, {
      blockedUrl: "https://evil.example/<script>alert(1)</script>",
    });

    expect(pageText()).toContain("evil.example");
    expect(pageText()).not.toContain("alert(1)");
    expect(risky()).toBe(before);
    expect(guard.violations).toEqual([]);
  });

  it("a_pushed_state_full_of_markup_adds_no_element", async () => {
    chrome.answerWith(focusState());
    await loadBlockPage(chrome);
    const before = risky();

    chrome.pushRaw({
      type: BLOCK_STATE,
      state: {
        phase: PAYLOAD,
        cycle: "<b>7</b>",
        cycles: PAYLOAD,
        remainingMs: PAYLOAD,
        asOf: NOW,
        strict: true,
        strictEndsAt: PAYLOAD,
      },
    });
    await flush(1_000);

    expect(risky()).toBe(before);
    expect(document.querySelector("b")).toBeNull();
    expect(Reflect.get(globalThis, XSS_FLAG)).toBeUndefined();
    expect(guard.violations).toEqual([]);
  });

  it("the_page_has_no_inline_event_handler", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    for (const element of document.querySelectorAll("*")) {
      const inline = [...element.attributes].filter((attribute) =>
        attribute.name.toLowerCase().startsWith("on"),
      );
      expect(
        inline.map((attribute) => attribute.name),
        element.tagName,
      ).toEqual([]);
    }
  });
});

describe("an address the page cannot show", () => {
  it("an_address_that_is_not_http_names_no_site", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome, { blockedUrl: "javascript:alert(1)" });

    const text = pageText();
    expect(text).not.toContain("alert(1)");
    expect(text).not.toContain("javascript:");
    // The page still works: it shows the session's state.
    expect(text).toMatch(/\b14:32\b/);
  });

  it("a_missing_address_names_no_site", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome, { blockedUrl: null });

    expect(pageText()).toMatch(/\b14:32\b/);
    // Message keys are left out: only what the page wrote itself is checked.
    expect(withoutSentinels(pageText())).not.toMatch(/NaN|undefined|null/);
  });
});
