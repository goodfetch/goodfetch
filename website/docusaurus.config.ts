// Goodfetch's docs site at goodfetch.dev/docs (ADRs 0030 and 0097). It
// builds the source tree `xtask docs-site render` writes: one docs instance
// whose root holds the chooser, the release notes and the benchmark results,
// and whose `collie/` and `foxhound/` hold each Product's section. The build
// fails on what Docusaurus would only warn about (TESTING.md, Docs site).
import { existsSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type * as Preset from "@docusaurus/preset-classic";
import type { Config } from "@docusaurus/types";
import { themes } from "prism-react-renderer";
import llmsPlugin from "./src/llms-plugin.ts";
import { searchOptions } from "./src/search.ts";
import { directivesUsed, fenceLanguage } from "./src/strict-markdown.ts";
import { readVersions, versionsConfig } from "./src/versions.ts";

// The rendered tree, by its real path: webpack hands the Markdown loader each
// page's real path, and the docs plugin finds a page's metadata by the path
// it was given, so a `target/` that is a link would leave every page without
// its metadata.
const rendered = fileURLToPath(new URL("../target/docs-site/source", import.meta.url));
if (!existsSync(rendered)) {
  throw new Error(`${rendered} is missing; run \`pnpm run render\` first`);
}
const source = realpathSync(rendered);

const config: Config = {
  title: "Goodfetch",
  tagline: "Browser automation and crawling that run on your machine",
  url: "https://goodfetch.dev",
  baseUrl: "/docs/",
  trailingSlash: false,
  onBrokenLinks: "throw",
  onBrokenAnchors: "throw",
  onDuplicateRoutes: "throw",
  markdown: {
    format: "detect",
    hooks: { onBrokenMarkdownLinks: "throw", onBrokenMarkdownImages: "throw" },
  },
  i18n: { defaultLocale: "en", locales: ["en"] },
  plugins: [llmsPlugin],
  themes: [["@easyops-cn/docusaurus-search-local", { ...searchOptions }]],
  presets: [
    [
      "classic",
      {
        docs: {
          path: `${source}/docs`,
          sidebarPath: `${source}/sidebars.json`,
          routeBasePath: "/",
          ...versionsConfig(readVersions("versions.json")),
          beforeDefaultRemarkPlugins: [fenceLanguage],
          remarkPlugins: [directivesUsed],
        },
        blog: false,
        theme: { customCss: "./src/css/custom.css" },
      } satisfies Preset.Options,
    ],
  ],
  themeConfig: {
    colorMode: { respectPrefersColorScheme: true },
    navbar: {
      title: "Goodfetch",
      items: [
        { type: "custom-productSwitcher", position: "left" },
        { type: "docsVersionDropdown", position: "right" },
        {
          href: "https://github.com/goodfetch/goodfetch",
          label: "GitHub",
          position: "right",
        },
      ],
    },
    footer: {
      links: [
        {
          title: "Docs",
          items: [
            { label: "Choose a Product", to: "/" },
            { label: "Accessibility statement", to: "/accessibility" },
          ],
        },
      ],
    },
    prism: { theme: themes.github, darkTheme: themes.dracula },
  } satisfies Preset.ThemeConfig,
};

export default config;
