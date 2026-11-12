// SPDX-License-Identifier: MIT OR Apache-2.0

import { onScopeDispose } from "vue";

import { onServiceEvent } from "@/services/ipc";
import type { ServiceEvent } from "@/services/types";

/**
 * Calls `handler` for every service event while the calling component (or
 * effect scope) lives, and stops listening when it is disposed, even if that
 * happens before the listener finished registering.
 */
export function useServiceEvents(handler: (event: ServiceEvent) => void): void {
  let disposed = false;
  let stop: (() => void) | undefined;

  onServiceEvent(handler).then(
    (unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        stop = unlisten;
      }
    },
    () => {
      // Without the event stream the data still loads; it only stops
      // refreshing on its own. Nothing to undo.
    },
  );

  onScopeDispose(() => {
    disposed = true;
    stop?.();
    stop = undefined;
  });
}
