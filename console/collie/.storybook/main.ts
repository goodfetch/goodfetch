// Storybook for the Console (ADR 0068; TESTING.md, Components), Collie's
// screens and Foxhound's (`console/foxhound`): each state of a component is
// a story, which the root Vitest configuration's
// `component-stories` projects render as a test, with the a11y addon's axe
// pass, in light and dark. Docgen is `react-docgen`, which needs no
// TypeScript compiler API.
//
// The Vitest addon builds stories from Storybook's own Vite configuration,
// without the package's `vite.config.ts`, so the design system's Tailwind
// plugin joins here when that file's has not: axe then checks the colours
// the build ships.
import tailwindcss from "@tailwindcss/vite";
import type { StorybookConfig } from "@storybook/react-vite";
import type { PluginOption } from "vite";

/** Whether `plugins`, a plugin or a list of them at any depth, holds Tailwind's. */
function hasTailwind(plugins: PluginOption | readonly PluginOption[]): boolean {
  if (Array.isArray(plugins)) {
    return plugins.some((plugin: PluginOption) => hasTailwind(plugin));
  }
  return (
    typeof plugins === "object" &&
    plugins !== null &&
    "name" in plugins &&
    typeof plugins.name === "string" &&
    plugins.name.startsWith("@tailwindcss/vite")
  );
}

const config: StorybookConfig = {
  framework: "@storybook/react-vite",
  // Foxhound's screens' stories render here too, so every Console story
  // takes one set of `component-stories` projects; Storybook is no build,
  // so Collie's build still never imports Foxhound's package (ADR 0087).
  stories: ["../src/**/*.stories.tsx", "../../foxhound/src/**/*.stories.tsx"],
  addons: ["@storybook/addon-a11y", "@storybook/addon-vitest"],
  typescript: { reactDocgen: "react-docgen" },
  core: { disableTelemetry: true },
  viteFinal: (vite) => {
    const plugins = vite.plugins ?? [];
    return hasTailwind(plugins) ? vite : { ...vite, plugins: [...plugins, tailwindcss()] };
  },
};

export default config;
