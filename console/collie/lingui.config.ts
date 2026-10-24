// The Console's message catalog (ADR 0073): English source messages,
// extracted from the Lingui macros by `lingui extract` and compiled ahead of
// time into the build by the Vite plugin. The catalog is JSON in a
// `messages/` folder, where `xtask wording` reads it (ADR 0097).
import { defineConfig } from "@lingui/cli";
import { formatter } from "@lingui/format-json";

const config: ReturnType<typeof defineConfig> = defineConfig({
  sourceLocale: "en",
  locales: ["en", "en-XA", "ar-XB"],
  pseudoLocale: [
    { locale: "en-XA", prepend: "[", append: "]", extend: 0.35 },
    { locale: "ar-XB", prepend: "", append: "", rightToLeft: true },
  ],
  catalogs: [
    {
      path: "<rootDir>/src/messages/{locale}",
      include: ["<rootDir>/src"],
      exclude: ["**/*.test.ts", "**/*.test.tsx", "**/*.stories.tsx"],
    },
  ],
  format: formatter({ style: "minimal" }),
});

export default config;
