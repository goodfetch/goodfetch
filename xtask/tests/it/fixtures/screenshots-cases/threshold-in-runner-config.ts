import { defineConfig } from "@playwright/test";

export default defineConfig({
  testMatch: "**/*.spec.ts",
  expect: {
    toHaveScreenshot: { threshold: 0.3 },
  },
});
