// SPDX-License-Identifier: MIT OR Apache-2.0

// Acceptance tests for T-4.4: the block page ships as an extension page the
// browser can reach, without widening what the extension may do, and it is
// styled with the design tokens of docs/ui-stack.md in both themes. This file
// runs in Node.js: it reads the files the extension ships and builds it.
//
//  - AC-10 the page is registered in the manifest and the build ships it
//      the_manifest_declares_the_block_page_as_a_web_accessible_resource
//      the_manifest_exposes_nothing_but_the_block_page
//      the_manifest_asks_for_no_new_capability
//      a_content_security_policy_forbids_inline_and_remote_code
//      the_block_page_files_exist_where_the_manifest_says
//      every_new_source_file_carries_its_licence_header
//      the_build_ships_the_block_page_with_everything_it_references
//      the_build_still_ships_the_service_worker_and_the_locales
//  - AC-9 the markup carries no inline code (nothing a Manifest V3 page could
//    even run) and loads nothing from the network
//      the_markup_runs_no_inline_script
//      the_page_loads_nothing_from_the_network
//  - AC-11 the page uses the shared design tokens, in light and dark, and
//    keeps the focus outline (WCAG 2.2 AA)
//      the_stylesheet_defines_the_design_tokens_of_the_light_theme
//      the_stylesheet_defines_the_design_tokens_of_the_dark_theme
//      the_stylesheet_never_removes_the_focus_outline

import {
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "vite";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("../..", import.meta.url));
const srcDir = join(root, "src");

/** Where the page lives in the source tree, and in the built extension. */
const PAGE = "block-page/index.html";
const PAGE_ENTRY = join(srcDir, "block-page", "block-page.ts");

interface WebAccessibleResource {
  resources?: string[];
  matches?: string[];
}

const manifest = JSON.parse(
  readFileSync(join(srcDir, "manifest.json"), "utf-8"),
) as Record<string, unknown> & {
  web_accessible_resources?: WebAccessibleResource[];
  content_security_policy?: { extension_pages?: string };
};

function pageHtml(): string {
  return readFileSync(join(srcDir, PAGE), "utf-8");
}

function styleSheets(): string[] {
  const dir = join(srcDir, "block-page");
  if (!existsSync(dir)) {
    return [];
  }
  return readdirSync(dir)
    .filter((name) => name.endsWith(".css"))
    .map((name) => readFileSync(join(dir, name), "utf-8"));
}

/** The local files a piece of markup loads, as written in it. */
function references(html: string): string[] {
  return [...html.matchAll(/\s(?:src|href)\s*=\s*["']([^"']+)["']/g)]
    .map((match) => match[1] ?? "")
    .filter((value) => !/^(https?:|data:|#|chrome-extension:)/i.test(value));
}

describe("the manifest", () => {
  it("the_manifest_declares_the_block_page_as_a_web_accessible_resource", () => {
    const entries = manifest.web_accessible_resources ?? [];

    expect(entries.flatMap((entry) => entry.resources ?? [])).toContain(PAGE);
    // Only the sites that can be blocked need to reach it (T-1.4: keitora-core
    // blocks http and https only).
    for (const entry of entries) {
      for (const match of entry.matches ?? []) {
        expect(["http://*/*", "https://*/*"]).toContain(match);
      }
      expect(entry.matches ?? []).not.toEqual([]);
    }
  });

  it("the_manifest_exposes_nothing_but_the_block_page", () => {
    const resources = (manifest.web_accessible_resources ?? []).flatMap(
      (entry) => entry.resources ?? [],
    );

    for (const resource of resources) {
      expect(resource.startsWith("block-page/"), resource).toBe(true);
      expect(resource).not.toContain("..");
    }
    expect(resources).not.toContain("block-page/*.map");
  });

  it("the_manifest_asks_for_no_new_capability", () => {
    for (const key of [
      "permissions",
      "optional_permissions",
      "host_permissions",
      "optional_host_permissions",
      "content_scripts",
      "externally_connectable",
      "declarative_net_request",
    ]) {
      expect(Object.keys(manifest), key).not.toContain(key);
    }
    // Anything else new is added on purpose, as src/manifest.spec.ts says.
    expect(Object.keys(manifest).sort()).toEqual(
      [
        "background",
        "content_security_policy",
        "default_locale",
        "description",
        "manifest_version",
        "name",
        "version",
        "web_accessible_resources",
      ].filter((key) => key in manifest),
    );
  });

  it("a_content_security_policy_forbids_inline_and_remote_code", () => {
    const policy = manifest.content_security_policy?.extension_pages;
    if (policy === undefined) {
      return;
    }

    expect(policy).not.toMatch(/unsafe-inline|unsafe-eval|unsafe-hashes/);
    expect(policy).not.toMatch(/https?:/);
  });
});

describe("the page's files", () => {
  it("the_block_page_files_exist_where_the_manifest_says", () => {
    expect(existsSync(join(srcDir, PAGE)), PAGE).toBe(true);
    expect(existsSync(PAGE_ENTRY), PAGE_ENTRY).toBe(true);
    expect(styleSheets().length).toBeGreaterThan(0);
  });

  it("every_new_source_file_carries_its_licence_header", () => {
    const licence = "SPDX-License-Identifier: MIT OR Apache-2.0";

    expect(pageHtml().startsWith(`<!-- ${licence} -->`)).toBe(true);
    expect(pageHtml()).toMatch(/<!doctype html>/i);
    expect(readFileSync(PAGE_ENTRY, "utf-8").startsWith(`// ${licence}`)).toBe(
      true,
    );
    for (const sheet of styleSheets()) {
      expect(sheet.startsWith(`/* ${licence} */`)).toBe(true);
    }
  });

  it("the_markup_runs_no_inline_script", () => {
    const html = pageHtml();

    // Manifest V3 forbids it, and nothing on this page needs it.
    expect(html).not.toMatch(/<script(?![^>]*\ssrc=)/i);
    expect(html).not.toMatch(/\son[a-z]+\s*=/i);
    expect(html).not.toMatch(/javascript:/i);
  });

  it("the_page_loads_nothing_from_the_network", () => {
    expect(pageHtml()).not.toMatch(/\b(src|href)\s*=\s*["']https?:/i);
    for (const sheet of styleSheets()) {
      expect(sheet).not.toMatch(/url\(\s*["']?https?:/i);
      expect(sheet).not.toMatch(/@import\s+(url\()?["']?https?:/i);
    }
    // Everything it loads is part of the extension; the built page's files
    // are checked below.
    expect(references(pageHtml()).length).toBeGreaterThan(0);
  });
});

describe("the page's styles", () => {
  const sheet = (): string =>
    styleSheets()
      .join("\n")
      .toLowerCase()
      .replace(/\s*:\s*/g, ": ");

  /** A colour of docs/ui-stack.md, in its long or its short form. */
  function hasColour(css: string, value: string): boolean {
    const short =
      value[1] === value[2] && value[3] === value[4] && value[5] === value[6]
        ? `#${value[1] ?? ""}${value[3] ?? ""}${value[5] ?? ""}`
        : value;
    return css.includes(value) || css.includes(short);
  }

  it("the_stylesheet_defines_the_design_tokens_of_the_light_theme", () => {
    const css = sheet();

    for (const [token, value] of [
      ["--background", "#fafaf9"],
      ["--card", "#ffffff"],
      ["--foreground", "#18181b"],
      ["--muted-foreground", "#71717a"],
      ["--primary", "#e5484d"],
      ["--strict", "#f5a524"],
      ["--border", "#e4e4e7"],
    ]) {
      expect(css, token).toContain(token);
      expect(hasColour(css, value ?? ""), `${token}: ${value}`).toBe(true);
    }
  });

  it("the_stylesheet_defines_the_design_tokens_of_the_dark_theme", () => {
    const css = sheet();
    const start = css.indexOf("prefers-color-scheme: dark");

    expect(start, "no dark theme in the stylesheet").toBeGreaterThanOrEqual(0);
    expect(css, "no color-scheme declaration").toContain("color-scheme");
    const dark = css.slice(start);
    for (const value of [
      "#0c0c0e",
      "#16161a",
      "#f4f4f5",
      "#a1a1aa",
      "#f2555a",
      "#f7b955",
      "#27272a",
    ]) {
      expect(hasColour(dark, value), value).toBe(true);
    }
  });

  it("the_stylesheet_never_removes_the_focus_outline", () => {
    for (const style of styleSheets()) {
      expect(style).not.toMatch(/outline\s*:\s*(none|0)\b/i);
    }
  });
});

describe("the built extension", () => {
  let outDir: string;

  beforeAll(async () => {
    outDir = mkdtempSync(join(tmpdir(), "keitora-block-page-"));
    await build({
      root,
      logLevel: "silent",
      build: { outDir, emptyOutDir: true },
    });
  }, 120_000);

  afterAll(() => {
    rmSync(outDir, { recursive: true, force: true });
  });

  function builtFiles(): string[] {
    return readdirSync(outDir, { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) =>
        relative(outDir, join(entry.parentPath, entry.name)).replaceAll(
          "\\",
          "/",
        ),
      );
  }

  it("the_build_ships_the_block_page_with_everything_it_references", () => {
    expect(builtFiles()).toContain(PAGE);

    const html = readFileSync(join(outDir, PAGE), "utf-8");
    expect(html).not.toMatch(/<script(?![^>]*\ssrc=)/i);
    const loaded = references(html);
    expect(loaded.length).toBeGreaterThan(0);
    for (const reference of loaded) {
      const resolved = join(outDir, dirname(PAGE), reference);
      expect(existsSync(resolved), reference).toBe(true);
    }
  });

  it("the_build_still_ships_the_service_worker_and_the_locales", () => {
    const files = builtFiles();

    expect(files).toContain("manifest.json");
    expect(files).toContain("service-worker.js");
    expect(files).toContain("_locales/en/messages.json");
    expect(files).toContain("_locales/es/messages.json");

    const built = JSON.parse(
      readFileSync(join(outDir, "manifest.json"), "utf-8"),
    ) as { web_accessible_resources?: WebAccessibleResource[] };
    for (const entry of built.web_accessible_resources ?? []) {
      for (const resource of entry.resources ?? []) {
        expect(files, resource).toContain(resource);
      }
    }
  });
});
