<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<!--
  The Blocklists screen. The app shell (T-3.3) only gives it its name; the
  lists and their sites, apps and exceptions are T-3.7.
-->
<script setup lang="ts">
import { useI18n } from "vue-i18n";

const { t } = useI18n();
</script>

<template>
  <h1 class="font-heading text-2xl font-semibold">
    {{ t("screens.blocklists") }}
  </h1>
</template>
