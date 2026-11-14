<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<!--
  The app's sidebar: the product name and a link to each of the four
  screens, in the order of the roadmap. It is the shadcn-vue `sidebar`
  component installed in T-0.8, in its fixed form (`collapsible="none"`,
  docs/ui-stack.md: "Fixed 220 px sidebar"), so it has no control to hide it
  and stays on every screen.

  vue-router marks the open screen's link with `aria-current="page"`, which
  a screen reader announces, and `isActive` gives it the highlight of the
  mockups (docs/assets/mockups).
-->
<script setup lang="ts">
import {
  ChartColumnIcon,
  ListIcon,
  LockIcon,
  SlidersHorizontalIcon,
  TimerIcon,
} from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { RouterLink, useRoute } from "vue-router";

import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";

const { t } = useI18n();
const route = useRoute();

/** The screens of the MVP, in the order they are listed in the roadmap. */
const items = computed(() => [
  { name: "timer", label: t("screens.timer"), icon: TimerIcon },
  { name: "blocklists", label: t("screens.blocklists"), icon: ListIcon },
  { name: "history", label: t("screens.history"), icon: ChartColumnIcon },
  {
    name: "settings",
    label: t("screens.settings"),
    icon: SlidersHorizontalIcon,
  },
]);
</script>

<template>
  <Sidebar collapsible="none" class="border-sidebar-border h-full border-r">
    <SidebarHeader>
      <div class="flex items-center gap-2 px-2 py-3">
        <span
          class="bg-primary text-primary-foreground flex size-8 items-center justify-center rounded-lg"
        >
          <LockIcon class="size-4" aria-hidden="true" />
        </span>
        <span class="font-heading text-base font-semibold">
          {{ t("app.name") }}
        </span>
      </div>
    </SidebarHeader>
    <SidebarContent>
      <nav :aria-label="t('nav.label')">
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu class="gap-1">
              <SidebarMenuItem v-for="item in items" :key="item.name">
                <SidebarMenuButton
                  as-child
                  size="lg"
                  :is-active="route.name === item.name"
                >
                  <RouterLink :to="{ name: item.name }">
                    <component :is="item.icon" aria-hidden="true" />
                    <span>{{ item.label }}</span>
                  </RouterLink>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </nav>
    </SidebarContent>
  </Sidebar>
</template>
