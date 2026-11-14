<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<!--
  The window the app runs in, from top to bottom: the custom title bar, then
  the sidebar and the screen the router shows, side by side
  (docs/ui-stack.md, "Window"). That is also the order a screen reader reads
  and the keyboard walks.

  `SidebarInset` is the `<main>` landmark of the window, so the screen is
  the only thing in it; the sidebar sits outside it.
-->
<script setup lang="ts">
import { RouterView } from "vue-router";

import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { useTheme } from "@/composables/useTheme";

import AppSidebar from "./AppSidebar.vue";
import AppTitleBar from "./AppTitleBar.vue";

/**
 * The sidebar's fixed width (docs/ui-stack.md, "Window"). It overrides the
 * default of the generated component, which the app leaves untouched.
 */
const SIDEBAR_WIDTH = "220px";

// The window follows the theme of the operating system while it runs.
useTheme();
</script>

<template>
  <div class="bg-background text-foreground flex h-screen flex-col">
    <AppTitleBar />
    <SidebarProvider
      class="min-h-0 flex-1"
      :style="{ '--sidebar-width': SIDEBAR_WIDTH }"
    >
      <AppSidebar />
      <SidebarInset class="min-w-0 overflow-y-auto p-8">
        <RouterView />
      </SidebarInset>
    </SidebarProvider>
  </div>
</template>
