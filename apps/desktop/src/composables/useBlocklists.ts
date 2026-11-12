// SPDX-License-Identifier: MIT OR Apache-2.0

import { onScopeDispose } from "vue";
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
/** Events arriving within this many ms of each other trigger one reload. */
const EVENT_BURST_MS = 50;
/** A bound on the pages read, so a misbehaving answer can't loop forever. */
const MAX_PAGES = 1000;

/** Reads every page of the blocklists, or throws if the paging is not sane. */
async function listAllBlocklists(): Promise<BlocklistSummary[]> {
  const all: BlocklistSummary[] = [];
  let offset = 0;
  for (let pages = 0; pages < MAX_PAGES; pages += 1) {
    const page = await listBlocklists({ offset, limit: PAGE_LIMIT });
    all.push(...page.items);
    const next = page.next_offset;
    if (next === null) {
      return all;
    }
    // A next page must exist, be a real number and move forward; anything
    // else is a service that would make this loop spin or read nothing.
    if (!Number.isSafeInteger(next) || next <= offset) {
      throw new Error("blocklists: next_offset does not advance");
    }
    if (page.items.length === 0) {
      throw new Error("blocklists: an empty page claims there is more");
    }
    offset = next;
  }
  throw new Error("blocklists: too many pages");
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

  // A burst of events (an import, say) reloads the list once.
  let pending: ReturnType<typeof setTimeout> | undefined;
  useServiceEvents((event) => {
    if (event.type === "blocklist_changed" && pending === undefined) {
      pending = setTimeout(() => {
        pending = undefined;
        void refresh();
      }, EVENT_BURST_MS);
    }
  });
  onScopeDispose(() => {
    clearTimeout(pending);
  });

  return {
    ...query,
    upsertBlocklist: upsert.mutateAsync,
    deleteBlocklist: remove.mutateAsync,
  };
}
