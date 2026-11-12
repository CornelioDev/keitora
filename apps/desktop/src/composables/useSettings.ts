// SPDX-License-Identifier: MIT OR Apache-2.0

import { useMutation, useQuery, useQueryClient } from "@tanstack/vue-query";

import { getSettings, updateSettings } from "@/services/ipc";

import { queryKeys } from "./queryKeys";

/**
 * The user's settings, plus `updateSettings`, which replaces them and fetches
 * them again. It rejects with the service's error when the service refuses.
 */
export function useSettings() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.settings,
    queryFn: getSettings,
  });
  const update = useMutation({
    mutationFn: updateSettings,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.settings }),
  });

  return { ...query, updateSettings: update.mutateAsync };
}
