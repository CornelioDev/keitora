// SPDX-License-Identifier: MIT OR Apache-2.0

/**
 * The app's routes: one per screen of the MVP (docs/ui-stack.md, "Frontend
 * layout"). The window has no address bar, so the router keeps the history
 * in memory instead of in the webview's location.
 */

import { createMemoryHistory, createRouter, type Router } from "vue-router";

import BlocklistsPage from "@/pages/BlocklistsPage.vue";
import HistoryPage from "@/pages/HistoryPage.vue";
import SettingsPage from "@/pages/SettingsPage.vue";
import TimerPage from "@/pages/TimerPage.vue";

/** The screen the app opens on, and where unknown addresses end up. */
const HOME = { name: "timer" } as const;

/**
 * Creates the app's router. Each call builds a router of its own, with its
 * own history, so that starting the app twice (src/main.ts, a test) never
 * shares navigation state.
 */
export function createAppRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", redirect: HOME },
      { path: "/timer", name: "timer", component: TimerPage },
      { path: "/blocklists", name: "blocklists", component: BlocklistsPage },
      { path: "/history", name: "history", component: HistoryPage },
      { path: "/settings", name: "settings", component: SettingsPage },
      // An address that is no screen opens the timer rather than an empty
      // window. The app builds its own links, so this only catches mistakes.
      { path: "/:unknown(.*)*", redirect: HOME },
    ],
  });
}
