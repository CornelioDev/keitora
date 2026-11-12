// SPDX-License-Identifier: MIT OR Apache-2.0

import { effectScope } from "vue";
import { describe, expect, it, vi } from "vitest";

const hoisted = vi.hoisted(() => ({
  onServiceEvent: vi.fn(),
}));
vi.mock("@/services/ipc", () => ({ onServiceEvent: hoisted.onServiceEvent }));

import { useServiceEvents } from "./useServiceEvents";

describe("useServiceEvents", () => {
  it("releases a listener that registered after the scope was disposed", async () => {
    const unlisten = vi.fn();
    let resolve!: (stop: () => void) => void;
    hoisted.onServiceEvent.mockReturnValue(
      new Promise<() => void>((r) => {
        resolve = r;
      }),
    );
    const scope = effectScope();
    scope.run(() => {
      useServiceEvents(vi.fn());
    });

    scope.stop();
    resolve(unlisten);
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("keeps the app working when the event stream cannot be opened", async () => {
    hoisted.onServiceEvent.mockRejectedValue(new Error("no stream"));
    const scope = effectScope();

    expect(() => {
      scope.run(() => {
        useServiceEvents(vi.fn());
      });
    }).not.toThrow();
    await Promise.resolve();
    scope.stop();
  });
});
