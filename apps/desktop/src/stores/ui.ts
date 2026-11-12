// SPDX-License-Identifier: MIT OR Apache-2.0

import { defineStore } from "pinia";
import { ref } from "vue";

/**
 * What the user picked on the Timer page. UI state only: session, blocklist
 * and settings data come from the service through vue-query, and this store
 * never talks to it.
 */
export const useUiStore = defineStore("ui", () => {
  /** The blocklist chosen for the next session, if any. */
  const selectedBlocklistId = ref<string | null>(null);
  /** Whether the next session should be started strict. */
  const strictRequested = ref(false);

  function selectBlocklist(id: string | null): void {
    selectedBlocklistId.value = id;
  }

  function setStrictRequested(value: boolean): void {
    strictRequested.value = value;
  }

  return {
    selectedBlocklistId,
    strictRequested,
    selectBlocklist,
    setStrictRequested,
  };
});
