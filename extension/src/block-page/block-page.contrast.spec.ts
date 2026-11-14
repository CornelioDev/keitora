// SPDX-License-Identifier: MIT OR Apache-2.0

// Contrast of the strict notice's text, computed from the stylesheet itself:
// the token values of each theme, the tint of the notice's background and the
// colour of its text. WCAG 2.2 AA asks 4.5:1 for text of normal size, and the
// notice's text is 15 px, regular weight.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const css = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "block-page.css"),
  "utf-8",
);

const DARK_QUERY = "@media (prefers-color-scheme: dark)";

type Tokens = Record<string, string>;
type Rgb = [number, number, number];

/** The custom properties declared in a piece of CSS. */
function declarations(source: string): Tokens {
  const tokens: Tokens = {};
  for (const [, name, value] of source.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    if (name !== undefined && value !== undefined) {
      tokens[name] = value.trim();
    }
  }
  return tokens;
}

/** The tokens of each theme: the dark one overrides the light one. */
function themes(): { light: Tokens; dark: Tokens } {
  const darkStart = css.indexOf(DARK_QUERY);
  expect(darkStart, "no dark theme").toBeGreaterThan(0);
  // The media block ends at the first closing brace in column 0.
  const darkEnd = css.indexOf("\n}", darkStart);
  const light = declarations(css.slice(0, darkStart));
  const dark = declarations(css.slice(darkStart, darkEnd));
  return { light, dark: { ...light, ...dark } };
}

/** A token's colour, following `var(--other)` references. */
function resolve(tokens: Tokens, value: string): Rgb {
  const reference = /^var\((--[\w-]+)\)$/.exec(value.trim());
  if (reference?.[1] !== undefined) {
    const target = tokens[reference[1]];
    expect(target, `${reference[1]} is not defined`).toBeDefined();
    return resolve(tokens, target ?? "");
  }
  const hex = /^#([0-9a-f]{6})$/i.exec(value.trim())?.[1];
  expect(hex, `${value} is not a #rrggbb colour`).toBeDefined();
  const digits = hex ?? "000000";
  return [0, 2, 4].map((at) =>
    Number.parseInt(digits.slice(at, at + 2), 16),
  ) as Rgb;
}

/** The declarations of the `.strict` rule. */
function strictRule(): string {
  const body = /\n\.strict\s*\{([^}]*)\}/.exec(css)?.[1];
  expect(body, "no .strict rule").toBeDefined();
  return body ?? "";
}

/** WCAG 2.2 relative luminance of an sRGB colour. */
function luminance([r, g, b]: Rgb): number {
  const [lr, lg, lb] = [r, g, b].map((channel) => {
    const c = channel / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as Rgb;
  return 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
}

function contrast(one: Rgb, other: Rgb): number {
  const [light, dark] = [luminance(one), luminance(other)].sort(
    (a, b) => b - a,
  ) as [number, number];
  return (light + 0.05) / (dark + 0.05);
}

/**
 * The notice's background as the reader sees it: `--strict` at the rule's
 * tint, over the page's `--background`, on which the notice sits.
 */
function strictBackground(tokens: Tokens): Rgb {
  // Anchored on `background`: the border is a stronger tint of the same colour.
  const tint =
    /(?:^|[;\s])background\s*:\s*color-mix\(in srgb, var\(--strict\) (\d+)%, transparent\)/.exec(
      strictRule(),
    )?.[1];
  expect(
    tint,
    "the notice's background is not a tint of --strict",
  ).toBeDefined();
  const share = Number(tint) / 100;
  const strict = resolve(tokens, "var(--strict)");
  const page = resolve(tokens, "var(--background)");
  return [0, 1, 2].map((i) => share * strict[i] + (1 - share) * page[i]) as Rgb;
}

/** The colour of the notice's text, as the rule declares it. */
function strictText(tokens: Tokens): Rgb {
  const colour = /(?:^|[;\s])color\s*:\s*([^;]+);/.exec(strictRule())?.[1];
  expect(colour, "the notice declares no text colour").toBeDefined();
  return resolve(tokens, colour ?? "");
}

describe("the strict notice's contrast", () => {
  it.each(["light", "dark"] as const)(
    "meets WCAG AA for its text in the %s theme",
    (theme) => {
      const tokens = themes()[theme];

      const ratio = contrast(strictText(tokens), strictBackground(tokens));

      expect(ratio, `${theme}: ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(
        4.5,
      );
    },
  );

  // Why the light theme does not write the text in --strict: the check above
  // would let a change back to it through only if this one failed too.
  it("would fail with --strict as the text colour of the light theme", () => {
    const tokens = themes().light;

    const ratio = contrast(
      resolve(tokens, "var(--strict)"),
      strictBackground(tokens),
    );

    expect(ratio).toBeLessThan(4.5);
  });

  it("matches the WCAG reference values", () => {
    expect(contrast([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 5);
    expect(contrast([118, 118, 118], [255, 255, 255])).toBeCloseTo(4.54, 2);
  });
});
