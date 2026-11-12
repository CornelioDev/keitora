// SPDX-License-Identifier: MIT OR Apache-2.0

// Runs a composable inside a component with the plugins the app installs
// (Pinia and vue-query), so the T-3.2 tests can use it like a page would.

import { QueryClient, VueQueryPlugin } from "@tanstack/vue-query";
import { createPinia } from "pinia";
import { mount, type VueWrapper } from "@vue/test-utils";
import { defineComponent, h } from "vue";

export function withSetup<T>(composable: () => T): {
  result: T;
  wrapper: VueWrapper;
  queryClient: QueryClient;
} {
  // No retries: a failing request must show as an error at once.
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  let result!: T;
  const wrapper = mount(
    defineComponent({
      setup() {
        result = composable();
        return () => h("div");
      },
    }),
    {
      global: {
        plugins: [createPinia(), [VueQueryPlugin, { queryClient }]],
      },
    },
  );
  return { result, wrapper, queryClient };
}
