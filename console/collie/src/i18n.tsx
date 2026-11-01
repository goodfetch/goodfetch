// The Console's messages (ADR 0073): the English catalog, compiled ahead of
// time by the Lingui Vite plugin when the build imports it, and the two
// pseudo-locales Lingui generates from it, `en-XA` (accented, longer,
// bracketed) and `ar-XB` (right to left), which a browser whose first
// language is one of them gets, each loaded only then. V1 ships in English:
// every other browser reads the English catalog, while numbers, dates and
// the rest are formatted in its own locale and time zone (`format`).
import { setupI18n, type I18n } from "@lingui/core";
import { I18nProvider } from "@lingui/react";
import { DirectionProvider } from "@base-ui/react/direction-provider";
import type { ReactNode } from "react";
import { messages as english } from "./messages/en.json?lingui";

/** The locales of the Console's catalogs: English and its two pseudo-locales. */
export type ConsoleLocale = "en" | "en-XA" | "ar-XB";

/** Which way a locale's text runs. */
export type TextDirection = "ltr" | "rtl";

/** The pseudo-locales, by their tag in lower case. */
const PSEUDO_LOCALES: Readonly<Record<string, ConsoleLocale>> = {
  "en-xa": "en-XA",
  "ar-xb": "ar-XB",
};

/** The catalog a browser whose languages are `languages`, most preferred first, reads. */
export function consoleLocale(languages: readonly string[]): ConsoleLocale {
  return PSEUDO_LOCALES[languages[0]?.toLowerCase() ?? ""] ?? "en";
}

/** Which way `locale`'s text runs. */
export function textDirection(locale: string): TextDirection {
  return locale === "ar-XB" ? "rtl" : "ltr";
}

/** The messages of `locale`'s catalog, a pseudo-locale's loaded on demand. */
async function catalogOf(locale: ConsoleLocale): Promise<I18n["messages"]> {
  switch (locale) {
    case "en":
      return english;
    case "en-XA":
      return (await import("./messages/en-XA.json?lingui")).messages;
    case "ar-XB":
      return (await import("./messages/ar-XB.json?lingui")).messages;
    default:
      return locale satisfies never;
  }
}

/** Gives `root`, the document's element, the language and direction of `locale`. */
export function applyLocale(root: HTMLElement, locale: string): void {
  root.lang = locale;
  root.dir = textDirection(locale);
}

const locale = consoleLocale(navigator.languages);

/** The Console's i18n instance, with the catalog of the browser's first language active. */
export const i18n: I18n = setupI18n({ locale, messages: { [locale]: await catalogOf(locale) } });

/** Gives every component below it the Console's messages and their direction. */
export function ConsoleI18n({ children }: { readonly children: ReactNode }): ReactNode {
  return (
    <I18nProvider i18n={i18n}>
      <DirectionProvider direction={textDirection(i18n.locale)}>{children}</DirectionProvider>
    </I18nProvider>
  );
}
