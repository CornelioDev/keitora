// SPDX-License-Identifier: MIT OR Apache-2.0

import { QueryClient, VueQueryPlugin } from "@tanstack/vue-query";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, h } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ServiceEvent } from "@/services/types";

const hoisted = vi.hoisted(() => ({
  listBlocklists: vi.fn(),
  emit: undefined as ((event: ServiceEvent) => void) | undefined,
}));
vi.mock("@/services/ipc", () => ({
  listBlocklists: hoisted.listBlocklists,
  upsertBlocklist: vi.fn(),
  deleteBlocklist: vi.fn(),
  onServiceEvent: vi.fn((handler: (event: ServiceEvent) => void) => {
    hoisted.emit = handler;
    return Promise.resolve(() => undefined);
  }),
}));

import { useBlocklists } from "./useBlocklists";

const ITEM = { id: "a", name: "A", color: "#000000" };

function load() {
  let result!: ReturnType<typeof useBlocklists>;
  const wrapper = mount(
    defineComponent({
      setup() {
        result = useBlocklists();
        return () => h("div");
      },
    }),
    {
      global: {
        plugins: [
          [
            VueQueryPlugin,
            {
              queryClient: new QueryClient({
                defaultOptions: { queries: { retry: false } },
              }),
            },
          ],
        ],
      },
    },
  );
  return { result, wrapper };
}

beforeEach(() => {
  hoisted.listBlocklists.mockReset();
  hoisted.emit = undefined;
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useBlocklists paging", () => {
  async function expectPagingError(second: unknown) {
    hoisted.listBlocklists
      .mockResolvedValueOnce({ items: [ITEM], next_offset: 1 })
      .mockResolvedValue(second);
    const { result } = load();
    await vi.waitFor(() => {
      expect(result.error.value).toBeTruthy();
    });
    expect(result.data.value).toBeUndefined();
  }

  it("fails when next_offset does not move forward", async () => {
    hoisted.listBlocklists.mockResolvedValue({ items: [ITEM], next_offset: 0 });
    const { result } = load();

    await vi.waitFor(() => {
      expect(result.error.value).toBeTruthy();
    });
    expect(hoisted.listBlocklists).toHaveBeenCalledTimes(1);
  });

  it("fails when next_offset goes backwards", async () => {
    await expectPagingError({ items: [ITEM], next_offset: 1 });
  });

  it("fails when next_offset is not a safe integer", async () => {
    for (const bad of [Number.NaN, 1.5, 2 ** 60, Infinity]) {
      hoisted.listBlocklists.mockReset();
      hoisted.listBlocklists.mockResolvedValue({
        items: [ITEM],
        next_offset: bad,
      });
      const { result, wrapper } = load();
      await vi.waitFor(() => {
        expect(result.error.value).toBeTruthy();
      });
      wrapper.unmount();
    }
  });

  it("fails when an empty page claims there is more", async () => {
    hoisted.listBlocklists
      .mockResolvedValueOnce({ items: [ITEM], next_offset: 1 })
      .mockResolvedValue({ items: [], next_offset: 2 });
    const { result } = load();

    await vi.waitFor(() => {
      expect(result.error.value).toBeTruthy();
    });
    expect(hoisted.listBlocklists).toHaveBeenCalledTimes(2);
  });

  it("fails after the page cap instead of reading forever", async () => {
    let offset = 0;
    hoisted.listBlocklists.mockImplementation(() => {
      offset += 1;
      return Promise.resolve({ items: [ITEM], next_offset: offset });
    });
    const { result } = load();

    await vi.waitFor(() => {
      expect(result.error.value).toBeTruthy();
    });
    expect(hoisted.listBlocklists).toHaveBeenCalledTimes(1000);
  });

  it("accepts an empty list on the last page", async () => {
    hoisted.listBlocklists.mockResolvedValue({ items: [], next_offset: null });
    const { result } = load();

    await vi.waitFor(() => {
      expect(result.data.value).toEqual([]);
    });
  });
});

describe("useBlocklists events", () => {
  it("reloads once for a burst of blocklist_changed events", async () => {
    hoisted.listBlocklists.mockResolvedValue({
      items: [ITEM],
      next_offset: null,
    });
    const { result } = load();
    await vi.waitFor(() => {
      expect(result.data.value).toEqual([ITEM]);
      expect(hoisted.emit).toBeDefined();
    });
    const before = hoisted.listBlocklists.mock.calls.length;

    vi.useFakeTimers();
    for (let i = 0; i < 20; i += 1) {
      hoisted.emit?.({ type: "blocklist_changed", id: "a" });
    }
    await vi.advanceTimersByTimeAsync(200);
    vi.useRealTimers();
    await flushPromises();

    expect(hoisted.listBlocklists.mock.calls.length).toBe(before + 1);
  });

  it("drops a pending reload when the component goes away", async () => {
    hoisted.listBlocklists.mockResolvedValue({
      items: [ITEM],
      next_offset: null,
    });
    const { result, wrapper } = load();
    await vi.waitFor(() => {
      expect(result.data.value).toEqual([ITEM]);
      expect(hoisted.emit).toBeDefined();
    });
    const before = hoisted.listBlocklists.mock.calls.length;

    vi.useFakeTimers();
    hoisted.emit?.({ type: "blocklist_changed", id: "a" });
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(200);
    vi.useRealTimers();
    await flushPromises();

    expect(hoisted.listBlocklists.mock.calls.length).toBe(before);
  });
});
