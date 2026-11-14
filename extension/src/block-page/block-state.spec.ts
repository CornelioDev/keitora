// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";

import {
  BLOCK_STATE,
  type BlockState,
  blockedHost,
  formatClock,
  formatTimeOfDay,
  isStateMessage,
  parseState,
  remainingAt,
} from "./block-state";

/** A state as the service worker sends it, with the overrides a test needs. */
function state(overrides: Partial<BlockState> = {}): BlockState {
  return {
    phase: "focus",
    cycle: 2,
    cycles: 4,
    remainingMs: 872_000,
    asOf: 1_000_000,
    strict: true,
    strictEndsAt: 2_000_000,
    ...overrides,
  };
}

describe("isStateMessage", () => {
  it("accepts a message of the contract", () => {
    expect(isStateMessage({ type: BLOCK_STATE, state: null })).toBe(true);
  });

  it.each([
    ["another type", { type: "keitora:something-else" }],
    ["no type", { state: null }],
    ["a string", "keitora:block-state"],
    ["null", null],
  ])("rejects %s", (_name, message) => {
    expect(isStateMessage(message)).toBe(false);
  });
});

describe("parseState", () => {
  it("reads a well-formed state", () => {
    expect(parseState(state())).toEqual(state());
  });

  it("reads the three phases", () => {
    for (const phase of ["focus", "short_break", "long_break"] as const) {
      expect(parseState(state({ phase }))?.phase).toBe(phase);
    }
  });

  it("accepts a strict session with no end time", () => {
    expect(parseState(state({ strictEndsAt: null }))?.strictEndsAt).toBeNull();
  });

  it("clamps a negative remaining time to zero", () => {
    expect(parseState(state({ remainingMs: -5_000 }))?.remainingMs).toBe(0);
  });

  it.each([
    ["no session", null],
    ["a string", "not a state"],
    ["an unknown phase", { ...state(), phase: "siesta" }],
    ["a cycle that is not a number", { ...state(), cycle: "<b>7</b>" }],
    ["a cycle of zero", { ...state(), cycle: 0 }],
    ["a fractional cycle", { ...state(), cycle: 1.5 }],
    ["no cycles at all", { ...state(), cycles: 0 }],
    ["a remaining time that is text", { ...state(), remainingMs: "soon" }],
    ["a reading time that is not a number", { ...state(), asOf: Number.NaN }],
    [
      "an infinite reading time",
      { ...state(), asOf: Number.POSITIVE_INFINITY },
    ],
    ["a strict flag that is not a boolean", { ...state(), strict: "yes" }],
    ["an end time that is not a number", { ...state(), strictEndsAt: "14:10" }],
    ["an end time of NaN", { ...state(), strictEndsAt: Number.NaN }],
  ])("refuses %s", (_name, value) => {
    expect(parseState(value)).toBeNull();
  });
});

describe("blockedHost", () => {
  it("takes the host of a percent-encoded address", () => {
    const search = `?url=${encodeURIComponent("https://news.example.org/a/b?secret=1")}`;

    expect(blockedHost(search)).toBe("news.example.org");
  });

  it("takes the host of an address that is not encoded", () => {
    expect(blockedHost("?url=https://videos.example.net/watch?v=abc")).toBe(
      "videos.example.net",
    );
  });

  it("keeps the port, which is part of the host", () => {
    expect(blockedHost("?url=http://localhost:8080/x")).toBe("localhost:8080");
  });

  it.each([
    ["no parameter at all", ""],
    ["an empty parameter", "?url="],
    ["another parameter", "?site=example.org"],
    ["an address that is not a URL", "?url=not a url"],
    ["a scheme that is never blocked", "?url=javascript:alert(1)"],
    ["a file address", "?url=file:///c:/secret.txt"],
    ["an extension page", "?url=chrome-extension://abc/page.html"],
  ])("names no site for %s", (_name, search) => {
    expect(blockedHost(search)).toBeNull();
  });

  it("names no site for markup in the parameter", () => {
    expect(blockedHost(`?url=<img src=x onerror="alert(1)">`)).toBeNull();
  });
});

describe("remainingAt", () => {
  it("discounts the time since the service worker read the state", () => {
    expect(remainingAt(state({ remainingMs: 872_000 }), 1_030_000)).toBe(
      842_000,
    );
  });

  it("never goes below zero", () => {
    expect(remainingAt(state({ remainingMs: 10_000 }), 9_000_000)).toBe(0);
  });

  it("adds no time when the state is read in the future", () => {
    expect(remainingAt(state({ remainingMs: 10_000 }), 999_000)).toBe(10_000);
  });
});

describe("formatClock", () => {
  it.each([
    [872_000, "14:32"],
    [65_000, "01:05"],
    [9_000, "00:09"],
    [0, "00:00"],
    [-5_000, "00:00"],
    [7_200_000, "120:00"],
  ])("writes %i ms as %s", (ms, expected) => {
    expect(formatClock(ms)).toBe(expected);
  });

  it("rounds down to the second that is still running", () => {
    expect(formatClock(1_999)).toBe("00:01");
  });
});

describe("formatTimeOfDay", () => {
  const endsAt = new Date(2026, 0, 2, 14, 10, 0).getTime();

  it("writes a time the way the language does", () => {
    expect(formatTimeOfDay(endsAt, "es")).toContain("14:10");
    expect(formatTimeOfDay(endsAt, "en")).toMatch(/\b2:10\b/);
  });

  it.each([
    ["no end time", null],
    ["a time that is not a number", Number.NaN],
  ])("writes nothing for %s", (_name, value) => {
    expect(formatTimeOfDay(value, "en")).toBe("");
  });

  it("writes nothing for a language it does not know", () => {
    expect(formatTimeOfDay(endsAt, "not a language")).toBe("");
  });
});
