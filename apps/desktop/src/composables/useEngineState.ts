// SPDX-License-Identifier: MIT OR Apache-2.0

import { useQuery, useQueryClient } from "@tanstack/vue-query";

import { getState } from "@/services/ipc";
import type { SessionState } from "@/services/types";

import { queryKeys } from "./queryKeys";
import { useServiceEvents } from "./useServiceEvents";

/**
 * The state of the session, as the service reports it. A `state_changed`
 * event replaces it at once; `session_ended` fetches it again.
 */
export function useEngineState() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.engineState,
    queryFn: getState,
  });

  useServiceEvents((event) => {
    if (event.type === "state_changed") {
      queryClient.setQueryData<SessionState>(
        queryKeys.engineState,
        event.state,
      );
    } else if (event.type === "session_ended") {
      void queryClient.invalidateQueries({ queryKey: queryKeys.engineState });
    }
  });

  return query;
}
