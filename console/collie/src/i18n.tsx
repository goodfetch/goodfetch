// The Console's messages (ADR 0073): the English catalog, compiled ahead of
// time by the Lingui Vite plugin when the build imports it.
import { setupI18n, type I18n } from "@lingui/core";
import { I18nProvider } from "@lingui/react";
import type { ReactNode } from "react";
import { messages } from "./messages/en.json?lingui";

/** The Console's i18n instance, with English active. */
export const i18n: I18n = setupI18n({ locale: "en", messages: { en: messages } });

/** Gives every component below it the Console's messages. */
export function ConsoleI18n({ children }: { readonly children: ReactNode }): ReactNode {
  return <I18nProvider i18n={i18n}>{children}</I18nProvider>;
}
