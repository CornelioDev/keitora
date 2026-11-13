// SPDX-License-Identifier: MIT OR Apache-2.0
// @vitest-environment jsdom

// Acceptance tests for T-4.4: the block page is localized. The extension
// localizes through `chrome.i18n` and `_locales`, not vue-i18n, and AGENTS.md
// allows no hard-coded user-facing string. The fake `chrome.i18n` answers
// every key with `[[key]]`, so any word on the page that is not inside such a
// stand-in is a string the page wrote itself.
//
//  - AC-8 every visible string comes from `chrome.i18n`, in both locales, and
//    the page declares the language of its text
//      every_visible_string_comes_from_chrome_i18n
//      every_visible_string_comes_from_chrome_i18n_without_data
//      localizes_the_page_title
//      declares_the_language_of_its_text_in_english
//      declares_the_language_of_its_text_in_spanish
//      the_shipped_markup_carries_no_user_visible_text
//      asks_for_at_least_one_message
//      every_message_the_page_uses_exists_in_english_and_in_spanish
//      the_two_locales_define_the_same_keys
//      no_message_the_page_uses_is_empty
//      every_message_the_page_uses_has_a_description_for_translators
//      a_message_with_substitutions_declares_them_in_both_locales

import { readFileSync } from "node:fs";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BLOCKED_HOST,
  ChromeFake,
  type I18nCall,
  NOW,
  focusState,
  loadBlockPage,
  pageText,
  readPageHtml,
  relaxedState,
  withoutSentinels,
} from "./support/t4-4-block-page";

interface LocaleMessage {
  message: string;
  description?: string;
  placeholders?: Record<string, { content: string }>;
}
type LocaleMessages = Record<string, LocaleMessage>;

function readMessages(locale: string): LocaleMessages {
  return JSON.parse(
    readFileSync(
      new URL(`../../src/_locales/${locale}/messages.json`, import.meta.url),
      "utf-8",
    ),
  ) as LocaleMessages;
}

/**
 * Words the page may show without a message of its own: the host of the
 * blocked site, and the AM/PM marker a locale adds to a clock time.
 */
function unexplainedWords(text: string): string {
  return withoutSentinels(text)
    .replaceAll(BLOCKED_HOST, " ")
    .replace(/\b[ap]\.?\s?m\.?\b/gi, " ");
}

/** Every message key the page uses, across the states it can be in. */
async function keysOfEveryState(): Promise<I18nCall[]> {
  const chrome = new ChromeFake();
  const states = [
    focusState(),
    relaxedState(),
    focusState({ phase: "short_break" }),
    focusState({ phase: "long_break" }),
    null,
  ];
  for (const state of states) {
    chrome.answerWith(state);
    await loadBlockPage(chrome);
  }
  chrome.failWith();
  await loadBlockPage(chrome);
  return chrome.i18nCalls;
}

let chrome: ChromeFake;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  chrome = new ChromeFake();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the page's own text", () => {
  it("every_visible_string_comes_from_chrome_i18n", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(unexplainedWords(pageText())).not.toMatch(/\p{L}/u);
  });

  it("every_visible_string_comes_from_chrome_i18n_without_data", async () => {
    chrome.failWith();

    await loadBlockPage(chrome);

    expect(unexplainedWords(pageText())).not.toMatch(/\p{L}/u);
  });

  it("localizes_the_page_title", async () => {
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(document.title).toMatch(/\[\[.+?\]\]/);
    expect(unexplainedWords(document.title)).not.toMatch(/\p{L}/u);
  });

  it("declares_the_language_of_its_text_in_english", async () => {
    chrome.uiLanguage = "en-GB";
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(document.documentElement.lang.toLowerCase()).toMatch(/^en\b/);
  });

  it("declares_the_language_of_its_text_in_spanish", async () => {
    chrome.uiLanguage = "es-419";
    chrome.answerWith(focusState());

    await loadBlockPage(chrome);

    expect(document.documentElement.lang.toLowerCase()).toMatch(/^es\b/);
  });

  it("the_shipped_markup_carries_no_user_visible_text", () => {
    const parsed = new DOMParser().parseFromString(readPageHtml(), "text/html");

    expect(parsed.body.textContent ?? "").not.toMatch(/\p{L}/u);
    for (const element of parsed.querySelectorAll("*")) {
      for (const name of ["alt", "title", "aria-label", "placeholder"]) {
        expect(element.getAttribute(name) ?? "", name).not.toMatch(/\p{L}/u);
      }
    }
  });
});

describe("the message files", () => {
  it("asks_for_at_least_one_message", async () => {
    const calls = await keysOfEveryState();

    expect(calls.length).toBeGreaterThan(0);
  });

  it("every_message_the_page_uses_exists_in_english_and_in_spanish", async () => {
    const calls = await keysOfEveryState();

    for (const locale of ["en", "es"]) {
      const messages = readMessages(locale);
      for (const { key } of calls) {
        expect(messages, `${locale} is missing "${key}"`).toHaveProperty(key);
      }
    }
  });

  it("the_two_locales_define_the_same_keys", () => {
    expect(Object.keys(readMessages("es")).sort()).toEqual(
      Object.keys(readMessages("en")).sort(),
    );
  });

  it("no_message_the_page_uses_is_empty", async () => {
    const calls = await keysOfEveryState();

    for (const locale of ["en", "es"]) {
      const messages = readMessages(locale);
      for (const { key } of calls) {
        expect(messages[key]?.message.trim(), `${locale} ${key}`).not.toBe("");
      }
    }
  });

  it("every_message_the_page_uses_has_a_description_for_translators", async () => {
    const calls = await keysOfEveryState();
    const messages = readMessages("en");

    for (const { key } of calls) {
      expect(messages[key]?.description?.trim(), key).toBeTruthy();
    }
  });

  it("a_message_with_substitutions_declares_them_in_both_locales", async () => {
    const calls = await keysOfEveryState();

    for (const locale of ["en", "es"]) {
      const messages = readMessages(locale);
      for (const { key, substitutions } of calls) {
        if (substitutions.length === 0) {
          continue;
        }
        const entry = messages[key];
        const declared =
          Object.keys(entry?.placeholders ?? {}).length >=
            substitutions.length ||
          substitutions.every((_value, index) =>
            (entry?.message ?? "").includes(`$${index + 1}`),
          );
        expect(declared, `${locale} ${key}`).toBe(true);
      }
    }
  });
});
