// Foxhound's Console's message catalogs (ADR 0073): Collie's, which its
// build compiles with Collie's screens, as Collie's own configuration
// extracts it, and Foxhound's, extracted from this package's screens. Both
// are English source messages in a `messages/` folder, compiled ahead of
// time into the build by the Vite plugin, where `xtask wording` reads them
// (ADR 0097).
import { defineConfig } from "@lingui/cli";
import { formatter } from "@lingui/format-json";

const exclude = ["**/*.test.ts", "**/*.test.tsx", "**/*.stories.tsx"];

const config: ReturnType<typeof defineConfig> = defineConfig({
  sourceLocale: "en",
  locales: ["en", "en-XA", "ar-XB"],
  pseudoLocale: [
    { locale: "en-XA", prepend: "[", append: "]", extend: 0.35 },
    { locale: "ar-XB", prepend: "", append: "", rightToLeft: true },
  ],
  catalogs: [
    {
      path: "<rootDir>/../collie/src/messages/{locale}",
      include: ["<rootDir>/../collie/src"],
      exclude,
    },
    {
      path: "<rootDir>/src/messages/{locale}",
      include: ["<rootDir>/src"],
      exclude,
    },
  ],
  format: formatter({ style: "minimal" }),
});

export default config;
