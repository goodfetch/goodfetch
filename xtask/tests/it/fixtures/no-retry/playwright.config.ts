import { defineConfig } from "@playwright/test";

export default defineConfig({
  testMatch: "**/*.spec.ts",
  retries: 0,
  failOnFlakyTests: true,
});
