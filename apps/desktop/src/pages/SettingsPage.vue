<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<!--
  The Settings screen. The app shell (T-3.3) only gives it its name; the
  durations, the behavior switches and the service status are T-3.9.
-->
<script setup lang="ts">
import { useI18n } from "vue-i18n";

const { t } = useI18n();
</script>

<template>
  <h1 class="font-heading text-2xl font-semibold">
    {{ t("screens.settings") }}
  </h1>
</template>
