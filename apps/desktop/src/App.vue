<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<!--
  Root component. In production builds it is the app shell
  (src/components/layout/AppShell.vue): the title bar, the sidebar and the
  screen the router shows.

  Development builds (`pnpm tauri dev`, `pnpm dev`) show the component
  gallery (src/dev/ComponentGallery.vue) instead, since the window has no
  address bar to open it with. Production builds replace
  `import.meta.env.DEV` with `false` and drop the dead branch, so the
  gallery and its import are left out of what ships.
-->
<script setup lang="ts">
import { defineAsyncComponent } from "vue";

import AppShell from "./components/layout/AppShell.vue";

const devPage = import.meta.env.DEV
  ? defineAsyncComponent(() => import("./dev/ComponentGallery.vue"))
  : undefined;
</script>

<template>
  <component :is="devPage" v-if="devPage" />
  <AppShell v-else />
</template>
