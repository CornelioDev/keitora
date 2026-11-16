// SPDX-License-Identifier: MIT OR Apache-2.0

import { readdirSync, readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

interface LocaleMessage {
  message: string;
  description?: string;
}
type LocaleMessages = Record<string, LocaleMessage>;

const srcDir = new URL("./", import.meta.url);

function readMessages(locale: string): LocaleMessages {
  const path = new URL(`_locales/${locale}/messages.json`, srcDir);
  return JSON.parse(readFileSync(path, "utf-8")) as LocaleMessages;
}

const manifestText = readFileSync(new URL("manifest.json", srcDir), "utf-8");
const manifest = JSON.parse(manifestText) as Record<string, unknown>;
const locales = readdirSync(new URL("_locales/", srcDir));

/** The keys of every `__MSG_key__` placeholder in the manifest. */
const manifestMessageKeys = [...manifestText.matchAll(/__MSG_(\w+?)__/g)].map(
  (match) => match[1],
);

describe("manifest.json", () => {
  it("is Manifest V3", () => {
    expect(manifest.manifest_version).toBe(3);
  });

  // An exact list, not a list of forbidden keys, so that any new permission,
  // host access, content script, `externally_connectable`, CSP or other
  // capability has to be added here on purpose. Phase F4 adds what it needs.
  it("declares only the skeleton's keys and asks for no capabilities", () => {
    expect(Object.keys(manifest).sort()).toEqual([
      "background",
      "content_security_policy",
      "default_locale",
      "description",
      "manifest_version",
      "name",
      "version",
      "web_accessible_resources",
    ]);
  });

  // T-4.4: the block page is the only resource a blocked site may reach, and
  // the policy leaves the page no way to run code that is not in the package.
  it("exposes the block page and nothing else", () => {
    expect(manifest.web_accessible_resources).toEqual([
      {
        resources: ["block-page/index.html"],
        matches: ["http://*/*", "https://*/*"],
      },
    ]);
  });

  it("allows no inline, remote or generated code on its pages", () => {
    const policy = manifest.content_security_policy as {
      extension_pages: string;
    };

    expect(policy.extension_pages).toBe("script-src 'self'; object-src 'self'");
  });

  it("runs its service worker as a module", () => {
    expect(manifest.background).toEqual({
      service_worker: "service-worker.js",
      type: "module",
    });
  });

  // The format browsers accept: one to four numbers from 0 to 65535, without
  // leading zeros.
  it("has a version that browsers accept", () => {
    const version = manifest.version as string;

    expect(version).toMatch(/^(0|[1-9]\d{0,4})(\.(0|[1-9]\d{0,4})){0,3}$/);
    expect(version.split(".").every((n) => Number(n) <= 65535)).toBe(true);
  });

  it("takes its visible name and description from the locale files", () => {
    expect(manifest.name).toMatch(/^__MSG_\w+__$/);
    expect(manifest.description).toMatch(/^__MSG_\w+__$/);
    expect(manifest.default_locale).toBe("en");
  });
});

describe("_locales", () => {
  it("has English and Spanish", () => {
    expect([...locales].sort()).toEqual(["en", "es"]);
  });

  it.each(locales)("%s has every message the manifest uses", (locale) => {
    const messages = readMessages(locale);

    expect(manifestMessageKeys.length).toBeGreaterThan(0);
    for (const key of manifestMessageKeys) {
      expect(messages, `missing "${key}"`).toHaveProperty(key);
    }
  });

  it.each(locales)("%s has the same keys as the default locale", (locale) => {
    const defaultKeys = Object.keys(readMessages("en")).sort();

    expect(Object.keys(readMessages(locale)).sort()).toEqual(defaultKeys);
  });

  it.each(locales)("%s has no empty message", (locale) => {
    for (const { message } of Object.values(readMessages(locale))) {
      expect(message.trim()).not.toBe("");
    }
  });

  // The limits Chrome documents for these two manifest fields.
  it.each(locales)("%s respects the manifest's length limits", (locale) => {
    const messages = readMessages(locale);

    expect(messages.extensionName.message.length).toBeLessThanOrEqual(75);
    expect(messages.extensionDescription.message.length).toBeLessThanOrEqual(
      132,
    );
  });

  it.each(locales)("%s calls the product Keitora", (locale) => {
    expect(readMessages(locale).extensionName.message).toBe("Keitora");
  });
});
