// SPDX-License-Identifier: MIT OR Apache-2.0

import { QueryClient } from "@tanstack/vue-query";

/**
 * The app's vue-query client. Every call goes to the local service through
 * Tauri, not over the network, so queries and mutations run whatever the
 * operating system says about connectivity. The default (`online`) would
 * pause queries and hold mutations such as `stop_session` while the machine
 * reports being offline.
 */
export function createAppQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { networkMode: "always" },
      mutations: { networkMode: "always" },
    },
  });
}
