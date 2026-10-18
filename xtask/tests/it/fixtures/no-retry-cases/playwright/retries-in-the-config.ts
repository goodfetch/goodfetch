import { defineConfig } from "@playwright/test";

export default defineConfig({
  testMatch: "**/*.spec.ts",
  retries: process.env.CI ? 2 : 0,
  failOnFlakyTests: true,
});
