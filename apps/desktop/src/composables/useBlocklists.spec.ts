// SPDX-License-Identifier: MIT OR Apache-2.0

import { QueryClient, VueQueryPlugin } from "@tanstack/vue-query";
import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, h } from "vue";
import { describe, expect, it, vi } from "vitest";

const hoisted = vi.hoisted(() => ({
  listBlocklists: vi.fn(),
}));
vi.mock("@/services/ipc", () => ({
  listBlocklists: hoisted.listBlocklists,
  upsertBlocklist: vi.fn(),
  deleteBlocklist: vi.fn(),
  onServiceEvent: vi.fn(() => Promise.resolve(() => undefined)),
}));

import { useBlocklists } from "./useBlocklists";

function load() {
  let result!: ReturnType<typeof useBlocklists>;
  mount(
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
  return result;
}

describe("useBlocklists paging", () => {
  it("stops when the service points to a page that does not move forward", async () => {
    hoisted.listBlocklists.mockResolvedValue({
      items: [{ id: "a", name: "A", color: "#000000" }],
      next_offset: 0,
    });

    const result = load();
    await flushPromises();

    expect(hoisted.listBlocklists).toHaveBeenCalledTimes(1);
    expect(result.data.value).toHaveLength(1);
  });
});
