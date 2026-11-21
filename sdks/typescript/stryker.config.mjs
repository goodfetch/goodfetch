// StrykerJS's settings for the TypeScript SDK's full mutation run (ADR 0072;
// TESTING.md, Feature coverage), run from the repository's root by
// `just mutants-sdks typescript`, weekly once the nightly tier is on. It
// mutates the hand-written runtime of the three packages, never what
// `xtask codegen` writes (ADR 0099), whose tests are its drift check and the
// generator's own, as `.cargo/mutants.toml` does for Rust.
//
// Vitest's runner, since StrykerJS 10.1.0 fixed stryker-js#6210, runs only
// the SDK's tests that reach each mutant. A surviving mutant fails nothing:
// no threshold breaks the run (ADR 0072, no mutation score is gated).

/** @type {import("@stryker-mutator/api/core").PartialStrykerOptions} */
const config = {
  mutate: ["sdks/typescript/*/src/**/*.ts", "!**/generated/**", "!**/*.test.ts", "!**/*.test-d.ts"],
  testRunner: "vitest",
  plugins: ["@stryker-mutator/vitest-runner"],
  // Forks, as `just test` runs them: some tests change the working directory,
  // which StrykerJS's default pool, threads, refuses.
  vitest: {
    configFile: "sdks/typescript/vitest.stryker.config.ts",
    dir: "sdks/typescript",
    related: true,
    pool: "forks",
  },
  // The checkout is the run's own: a copy of the repository and its build
  // folder would cost more than the mutants, as for cargo-mutants'
  // `--in-place`. Each mutated file is restored when the run ends. StrykerJS
  // reads nothing outside the SDK, and writes only the files it mutates:
  // Vitest checks no types, so `disableTypeChecks` stays off: StrykerJS
  // would otherwise write a type-check opt-out into every script of the
  // checkout.
  inPlace: true,
  ignorePatterns: ["/*", "!/sdks", "/sdks/*", "!/sdks/typescript"],
  disableTypeChecks: false,
  tempDirName: "target/stryker-tmp",
  thresholds: { high: 80, low: 60, break: null },
  reporters: ["clear-text", "progress", "html", "json"],
  htmlReporter: { fileName: "target/mutants-sdks/typescript/report.html" },
  jsonReporter: { fileName: "target/mutants-sdks/typescript/report.json" },
};

export default config;
