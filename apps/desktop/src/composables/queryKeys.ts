// SPDX-License-Identifier: MIT OR Apache-2.0

// The vue-query keys of the service's data, shared by the composables that
// read it and the ones that invalidate it.

export const queryKeys = {
  engineState: ["engine-state"] as const,
  blocklists: ["blocklists"] as const,
  settings: ["settings"] as const,
};
