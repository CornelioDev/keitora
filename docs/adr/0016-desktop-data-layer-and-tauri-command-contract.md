# 0016 — Desktop data layer and the Tauri command contract

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

T-3.2 builds the frontend's data layer before T-3.1 builds the Rust IPC client in `src-tauri`, so the frontend had to fix the names T-3.1 will register. The UI is an untrusted client ([ADR-0001](0001-privileged-service-as-source-of-truth.md)): it must never hold state the service owns, and every call goes to a local pipe, not the network. The roadmap gave no acceptance criteria for T-3.2; QA derived them from the task, `docs/ui-stack.md` and `docs/architecture.md`.

## Decision

- **One seam.** `apps/desktop/src/services/ipc.ts` is the only module that imports `@tauri-apps/api`. Components use the composables, which use it.
- **Tauri command contract**, which T-3.1 must register exactly:
  - Commands are the snake_case form of the requests in `docs/architecture.md`: `get_state`, `start_session`, `stop_session`, `pause_session`, `resume_session`, `skip_break`, `list_blocklists`, `get_blocklist`, `upsert_blocklist`, `delete_blocklist`, `get_settings`, `update_settings`.
  - Arguments are the camelCase form of the request fields (Tauri's convention), such as `blocklistId`, `preset`, `lock`, `text` (`null` when absent), `page`, `id`, `blocklist` and `settings`.
  - A failed command rejects with the service's `ProtocolError` object (`{ code, ... }`), passed on untouched, so `strict_session_locked` stays distinguishable.
  - Service events are forwarded under one Tauri event, `service-event`, with the event's flat payload (`{ type: "state_changed", state }`).
  - No request carries a time.
- **Server state in `@tanstack/vue-query`**: `useEngineState`, `useBlocklists` and `useSettings`. Mutations live on the composables (`upsertBlocklist`, `deleteBlocklist`, `updateSettings`) and refresh their query. Each composable subscribes to `service-event` itself and releases the listener when its scope is disposed. `state_changed` replaces the cached state, `session_ended` refetches it, and `blocklist_changed` reloads the blocklists once per burst. Blocklists are read in pages of 100, and bad paging (an offset that doesn't advance or isn't a safe integer, an empty page that claims more, more than 1000 pages) is an error, not a silent truncation.
- **`networkMode: "always"`** for queries and mutations: the default `online` mode would pause them, and hold a mutation such as `stop_session`, while Windows reports no network, although the pipe is local.
- **UI state in Pinia**: the `ui` store holds only `selectedBlocklistId` and `strictRequested`, a request flag. It holds no session, lock, remaining time or blocklist content, and never calls the service. The strict banner and the disabled Stop button come from `useEngineState().data.lock`, never from the store.
- **`vue-demi`'s install script stays off** (`allowBuilds: { vue-demi: false }`): it only rewires its own entry points for Vue 2, and the files it ships already target Vue 3.

## Alternatives considered

- **One shared event listener** (a plugin or `useServiceEvents()` at the app root): fewer listeners, but a global side effect; the per-composable listener is simpler and can be revisited if many components use the composables.
- **Server state in Pinia**: would duplicate caching, refetching and invalidation that vue-query already does.
- **Waiting for T-3.1's generated bindings**: would serialize the two fronts. The hand-written types in `services/types.ts` are replaced by the `tauri-specta` bindings in T-3.1.
- **Matching command names loosely**: the acceptance tests' fake does, so they don't fix the names; the names are fixed here instead, in one place.

## Follow-ups

- T-3.1: register the contract above, grant the main window only `core:event:allow-listen` (never `core:event:allow-emit`, so the frontend can't forge `service-event`), and check the shape of `state_changed` before it enters the cache (security review findings L1 and L2).
