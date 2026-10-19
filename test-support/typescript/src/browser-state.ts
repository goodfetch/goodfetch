// The browser state a Vitest Browser Mode test sets through the Playwright
// provider (ADR 0068; TESTING.md, Browser state), never by patching
// `matchMedia`, `navigator` or `Intl`. Each field left out keeps its value.

/** The browser state `browserState` sets; each field left out is unchanged. */
export interface BrowserState {
  /** `prefers-color-scheme`. */
  readonly colorScheme?: "light" | "dark" | "no-preference";
  /** `prefers-reduced-motion`. */
  readonly reducedMotion?: "reduce" | "no-preference";
  /** The locale `navigator.language` and `Intl` take, such as `tr-TR` (Chromium only). */
  readonly locale?: string;
  /** The IANA time zone `Intl` and `Date` take, such as `Pacific/Chatham` (Chromium only). */
  readonly timeZone?: string;
  /**
   * `navigator.onLine`, with its `online` and `offline` events (Chromium
   * only). The page's requests still reach the network, as a component test
   * makes none.
   */
  readonly online?: boolean;
}
