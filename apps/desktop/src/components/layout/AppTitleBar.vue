<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<!--
  The window's title bar, drawn by the app because the window has no native
  decorations (`decorations: false` in src-tauri/tauri.conf.json).

  The bar itself is the window's drag region: `data-tauri-drag-region` tells
  Tauri to move the window when it is dragged, which needs no code and no
  JavaScript permission beyond `core:window:allow-start-dragging`. The three
  controls are ordinary buttons, outside that region so that they are
  clicked rather than dragged.

  The controls use exactly three methods of the window — `minimize()`,
  `toggleMaximize()` and `close()` — each granted by one permission in
  src-tauri/capabilities/default.json. Anything else, such as asking whether
  the window is maximized to swap the middle icon, would need another
  permission, so the icon is the same whatever state the window is in.
-->
<script setup lang="ts">
import { MinusIcon, SquareIcon, XIcon } from "@lucide/vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// The window is reached when a control is used, never while the app starts:
// `getCurrentWindow()` needs Tauri's API, which only the webview of the app
// provides.
function minimize(): void {
  void getCurrentWindow().minimize();
}

function toggleMaximize(): void {
  void getCurrentWindow().toggleMaximize();
}

function close(): void {
  void getCurrentWindow().close();
}
</script>

<template>
  <header
    data-tauri-drag-region
    class="bg-background flex h-10 shrink-0 items-center justify-end select-none"
  >
    <button
      type="button"
      data-window-control="minimize"
      :aria-label="t('window.minimize')"
      class="hover:bg-accent hover:text-accent-foreground focus-visible:ring-ring inline-flex h-10 w-12 items-center justify-center outline-hidden focus-visible:ring-2 focus-visible:ring-inset"
      @click="minimize"
    >
      <MinusIcon class="size-4" aria-hidden="true" />
    </button>
    <button
      type="button"
      data-window-control="toggle-maximize"
      :aria-label="t('window.toggleMaximize')"
      class="hover:bg-accent hover:text-accent-foreground focus-visible:ring-ring inline-flex h-10 w-12 items-center justify-center outline-hidden focus-visible:ring-2 focus-visible:ring-inset"
      @click="toggleMaximize"
    >
      <SquareIcon class="size-3.5" aria-hidden="true" />
    </button>
    <button
      type="button"
      data-window-control="close"
      :aria-label="t('window.close')"
      class="hover:bg-destructive focus-visible:ring-ring inline-flex h-10 w-12 items-center justify-center outline-hidden hover:text-white focus-visible:ring-2 focus-visible:ring-inset"
      @click="close"
    >
      <XIcon class="size-4" aria-hidden="true" />
    </button>
  </header>
</template>
