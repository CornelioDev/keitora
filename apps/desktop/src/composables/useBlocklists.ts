// SPDX-License-Identifier: MIT OR Apache-2.0

import { useMutation, useQuery, useQueryClient } from "@tanstack/vue-query";

import {
  deleteBlocklist,
  listBlocklists,
  upsertBlocklist,
} from "@/services/ipc";
import type { BlocklistSummary } from "@/services/types";

import { queryKeys } from "./queryKeys";
import { useServiceEvents } from "./useServiceEvents";

/** The most items the protocol returns in one page. */
const PAGE_LIMIT = 100;
/** A bound on the pages read, so a misbehaving answer can't loop forever. */
const MAX_PAGES = 1000;

/** Reads every page of the blocklists. */
async function listAllBlocklists(): Promise<BlocklistSummary[]> {
  const all: BlocklistSummary[] = [];
  let offset = 0;
  for (let pages = 0; pages < MAX_PAGES; pages += 1) {
    const page = await listBlocklists({ offset, limit: PAGE_LIMIT });
    all.push(...page.items);
    // The next page must move forward.
    if (page.next_offset === null || page.next_offset <= offset) {
      break;
    }
    offset = page.next_offset;
  }
  return all;
}

/**
 * Every blocklist (summaries), plus the mutations that change them. The list
 * is fetched again after a mutation and on `blocklist_changed` events.
 * Mutations reject with the service's error, such as `strict_session_locked`.
 */
export function useBlocklists() {
  const queryClient = useQueryClient();
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: queryKeys.blocklists });

  const query = useQuery({
    queryKey: queryKeys.blocklists,
    queryFn: listAllBlocklists,
  });
  const upsert = useMutation({
    mutationFn: upsertBlocklist,
    onSuccess: refresh,
  });
  const remove = useMutation({
    mutationFn: deleteBlocklist,
    onSuccess: refresh,
  });

  useServiceEvents((event) => {
    if (event.type === "blocklist_changed") {
      void refresh();
    }
  });

  return {
    ...query,
    upsertBlocklist: upsert.mutateAsync,
    deleteBlocklist: remove.mutateAsync,
  };
}
