// Seam B (spec #326): each setting of the `browserState` browser command is
// read back from the page through the browser's own interfaces (`matchMedia`,
// `Intl`, `navigator`), and `browserStateReset`, which `browserState` runs
// once its test ends, gives the page back the project's state (ADR 0068).
// tests/browser-state-reset.test.ts proves the reset on a project whose
// colour scheme and motion are not Playwright's defaults.
import { describe, expect, test } from "vitest";
import {
  browserState,
  browserStateReset,
  type BrowserState,
} from "@goodfetch/test-support/vitest-browser";

interface Setting {
  readonly name: string;
  readonly state: BrowserState;
  readonly readBack: () => unknown;
  readonly expected: unknown;
}

const settings: readonly Setting[] = [
  {
    name: "colour scheme",
    state: { colorScheme: "dark" },
    readBack: () => matchMedia("(prefers-color-scheme: dark)").matches,
    expected: true,
  },
  {
    name: "reduced motion",
    state: { reducedMotion: "reduce" },
    readBack: () => matchMedia("(prefers-reduced-motion: reduce)").matches,
    expected: true,
  },
  {
    name: "locale",
    state: { locale: "tr-TR" },
    readBack: () => [
      navigator.language,
      navigator.languages,
      new Intl.NumberFormat().resolvedOptions().locale,
    ],
    expected: ["tr-TR", ["tr-TR"], "tr-TR"],
  },
  {
    name: "time zone",
    state: { timeZone: "Pacific/Chatham" },
    readBack: () => new Intl.DateTimeFormat().resolvedOptions().timeZone,
    expected: "Pacific/Chatham",
  },
  {
    name: "online status",
    state: { online: false },
    readBack: () => navigator.onLine,
    expected: false,
  },
];

function pageState(): Readonly<Record<string, unknown>> {
  return {
    dark: matchMedia("(prefers-color-scheme: dark)").matches,
    reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
    locale: new Intl.NumberFormat().resolvedOptions().locale,
    language: navigator.language,
    timeZone: new Intl.DateTimeFormat().resolvedOptions().timeZone,
    online: navigator.onLine,
  };
}

describe("browserState", () => {
  test.for(settings)("sets the $name", async ({ state, readBack, expected }) => {
    await browserState(state);

    expect(readBack()).toEqual(expected);
  });

  test("browserStateReset gives the page back the project's state", async () => {
    const before = pageState();
    await browserState({
      colorScheme: "dark",
      reducedMotion: "reduce",
      locale: "tr-TR",
      timeZone: "Pacific/Chatham",
      online: false,
    });

    await browserStateReset();

    expect(pageState()).toEqual(before);
  });
});
