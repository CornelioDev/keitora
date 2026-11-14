// SPDX-License-Identifier: MIT OR Apache-2.0

import {
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "vite";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { manifestVersion } from "./manifest-version";

const root = fileURLToPath(new URL("..", import.meta.url));

// Builds with the real vite.config.ts, into a temporary directory so that the
// test leaves dist/ alone.
describe("the build output", () => {
  let outDir: string;

  beforeAll(async () => {
    outDir = mkdtempSync(join(tmpdir(), "keitora-extension-build-"));
    await build({
      root,
      logLevel: "silent",
      build: { outDir, emptyOutDir: true },
    });
  });

  afterAll(() => {
    rmSync(outDir, { recursive: true, force: true });
  });

  it("contains exactly the files the browser needs", () => {
    const files = readdirSync(outDir, { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) =>
        relative(outDir, join(entry.parentPath, entry.name)).replaceAll(
          "\\",
          "/",
        ),
      )
      .sort();

    expect(files).toEqual([
      "_locales/en/messages.json",
      "_locales/es/messages.json",
      "block-page/block-page.css",
      "block-page/block-page.js",
      "block-page/index.html",
      "manifest.json",
      "service-worker.js",
    ]);
  });

  it("changes only the manifest's version, and its service worker exists", () => {
    const source = JSON.parse(
      readFileSync(new URL("manifest.json", import.meta.url), "utf-8"),
    ) as Record<string, unknown>;
    const { version: projectVersion } = JSON.parse(
      readFileSync(new URL("../package.json", import.meta.url), "utf-8"),
    ) as { version: string };
    const built = JSON.parse(
      readFileSync(join(outDir, "manifest.json"), "utf-8"),
    ) as { background: { service_worker: string } };

    expect(built).toEqual({
      ...source,
      version: manifestVersion(projectVersion),
    });
    expect(existsSync(join(outDir, built.background.service_worker))).toBe(
      true,
    );
  });
});
