// What every story of Collie's Console renders inside (ADR 0073; TESTING.md,
// Accessibility): the Console's messages and design system, in the colour
// scheme of the `theme` global, which each `component-stories` project
// pins; and axe through the a11y addon, every violation of WCAG 2.2 AA's
// tags and best practice failing the story's test.
import type { Decorator, Preview } from "@storybook/react-vite";
import { applyLocale, ConsoleI18n, i18n } from "#i18n.tsx";
import "#styles.css";

/** The colour scheme a story renders in, in the language and direction of the catalog the browser's locale picks, as the Console's entry gives its document. */
const withTheme: Decorator = (Story, context) => {
  const dark = context.globals["theme"] === "dark";
  document.documentElement.classList.toggle("dark", dark);
  applyLocale(document.documentElement, i18n.locale);
  return (
    <ConsoleI18n>
      <main className="bg-background p-6 text-foreground">
        <Story />
      </main>
    </ConsoleI18n>
  );
};

const preview: Preview = {
  decorators: [withTheme],
  initialGlobals: { theme: "light" },
  parameters: {
    a11y: {
      test: "error",
      options: {
        runOnly: {
          type: "tag",
          values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa", "best-practice"],
        },
      },
    },
  },
};

export default preview;
