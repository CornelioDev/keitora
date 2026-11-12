// SPDX-License-Identifier: MIT OR Apache-2.0

import { VueQueryPlugin } from "@tanstack/vue-query";
import { createPinia } from "pinia";
import { createApp } from "vue";
import App from "./App.vue";
import {
  createAppI18n,
  detectLocale,
  preferredLanguages,
  syncDocumentLanguage,
} from "./i18n";
import { createAppQueryClient } from "./queryClient";
import "./styles/main.css";

// The app starts in the user's language when it has translations for it
// (src/i18n.ts), and in English otherwise.
const i18n = createAppI18n(detectLocale(preferredLanguages(navigator)));

// Pinia holds UI state; vue-query caches the service data (src/composables).
const app = createApp(App)
  .use(i18n)
  .use(createPinia())
  .use(VueQueryPlugin, { queryClient: createAppQueryClient() });
const stopLanguageSync = syncDocumentLanguage(
  i18n.global.locale,
  document.documentElement,
);
app.onUnmount(stopLanguageSync);
app.mount("#app");
