// SPDX-License-Identifier: MIT OR Apache-2.0

import { usePreferredDark } from "@vueuse/core";
import { watch, type WatchHandle } from "vue";

/**
 * The class the design tokens (src/styles/tokens.css) use for the dark
 * theme. It belongs on the document's root element, `<html>`, and nowhere
 * else: every token is defined either on `:root` or on `.dark`.
 */
const DARK_CLASS = "dark";

/**
 * Follows the theme the operating system asks for, as
 * `prefers-color-scheme` reports it, by adding or removing the `dark` class
 * on `root` (the document's `<html>` element by default).
 *
 * Windows tells the webview about a theme change while the app runs, so the
 * window turns dark or light with the system, without a restart. The
 * setting that overrides this choice is T-3.9.
 *
 * Call it from a component's `setup()`: it stops following the system when
 * that component is unmounted. The returned handle stops it earlier.
 */
export function useTheme(
  root: HTMLElement = document.documentElement,
): WatchHandle {
  const prefersDark = usePreferredDark();

  // `sync` applies the theme as soon as the system changes it, rather than
  // on Vue's next render, so no frame is drawn in the wrong theme.
  return watch(
    prefersDark,
    (dark) => {
      root.classList.toggle(DARK_CLASS, dark);
    },
    { immediate: true, flush: "sync" },
  );
}
