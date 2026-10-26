// Foxhound's own messages (ADR 0073): its English catalog and the two
// pseudo-locales Lingui generates from it, each loaded only for a browser
// whose first language it is, which the build entry adds to the Console's.
import type { Messages } from "@lingui/core";

/** The messages of Foxhound's catalog in `locale`. */
export async function foxhoundMessages(locale: "en" | "en-XA" | "ar-XB"): Promise<Messages> {
  switch (locale) {
    case "en":
      return (await import("./messages/en.json?lingui")).messages;
    case "en-XA":
      return (await import("./messages/en-XA.json?lingui")).messages;
    case "ar-XB":
      return (await import("./messages/ar-XB.json?lingui")).messages;
    default:
      return locale satisfies never;
  }
}
