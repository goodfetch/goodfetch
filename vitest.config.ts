// Vitest for the whole pnpm workspace (TESTING.md: No retries, Before merge,
// Time budgets; ADRs 0061, 0062 and 0064). Unit tests sit beside their module
// and take the Quick suite's caps; integration tests under `tests/` take the
// caps above it. CI runs with `CI` set, so a missing or mismatched snapshot
// fails and none is written.
import { defineConfig, type ViteUserConfig } from "vitest/config";

const outside = ["**/node_modules/**", "rnd/**", "ci/proofs/**", "target/**"];

const config: ViteUserConfig = defineConfig({
  test: {
    retry: 0,
    restoreMocks: true,
    unstubEnvs: true,
    unstubGlobals: true,
    sequence: { shuffle: { tests: true } },
    passWithNoTests: true,
    // The Quick suite passes `--slowTestThreshold=1000`; Vitest sets it per run, not per project.
    slowTestThreshold: 30_000,
    reporters: ["default", "junit"],
    outputFile: { junit: "target/junit/vitest.xml" },
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          include: ["**/src/**/*.test.{ts,tsx}"],
          exclude: outside,
          testTimeout: 10_000,
        },
      },
      {
        extends: true,
        test: {
          name: "integration",
          include: ["**/tests/**/*.test.{ts,tsx}"],
          exclude: outside,
          testTimeout: 300_000,
        },
      },
    ],
  },
});

export default config;
