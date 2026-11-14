<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<!--
  The Timer screen. The app shell (T-3.3) only gives it its name; the ring,
  the presets and the Start button are T-3.4.
-->
<script setup lang="ts">
import { useI18n } from "vue-i18n";

const { t } = useI18n();
</script>

<template>
  <h1 class="font-heading text-2xl font-semibold">{{ t("screens.timer") }}</h1>
</template>
