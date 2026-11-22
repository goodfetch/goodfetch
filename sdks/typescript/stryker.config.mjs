// StrykerJS's settings for the TypeScript SDK's mutation runs (ADR 0072;
// TESTING.md, Feature coverage), run from the repository's root by
// ci/mutants-typescript.sh: the full run, `just mutants-sdks typescript`,
// weekly once the nightly tier is on, and the in-diff run,
// `just mutants-typescript`, whose `--mutate` list of the changed lines
// replaces `mutate` below. It mutates the hand-written runtime of the three
// packages, never what `xtask codegen` writes (ADR 0099), whose tests are
// its drift check and the generator's own, as `.cargo/mutants.toml` does for
// Rust. The script names the reports' folder (GOODFETCH_STRYKER_REPORTS).
//
// Vitest's runner, since StrykerJS 10.1.0 fixed stryker-js#6210, runs only
// the SDK's tests that reach each mutant, as each test's coverage shows,
// the Node programs it starts included (test support's
// `mutation-children.ts`). A surviving mutant fails nothing: no threshold
// breaks the run (ADR 0072, no mutation score is gated).

const reports = process.env.GOODFETCH_STRYKER_REPORTS ?? "target/mutants-sdks/typescript";

/** @type {import("@stryker-mutator/api/core").PartialStrykerOptions} */
const config = {
  mutate: ["sdks/typescript/*/src/**/*.ts", "!**/generated/**", "!**/*.test.ts", "!**/*.test-d.ts"],
  testRunner: "vitest",
  plugins: ["@stryker-mutator/vitest-runner"],
  // Forks, as `just test` runs them: some tests change the working directory,
  // which StrykerJS's default pool, threads, refuses. Every test of the SDK
  // runs once to learn its coverage, not only those whose imports reach a
  // mutated file (`related`): a test that reaches a file only through the
  // program it starts, such as `collie/src/bin.ts`, imports none.
  vitest: {
    configFile: "sdks/typescript/vitest.stryker.config.ts",
    dir: "sdks/typescript",
    related: false,
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
  // StrykerJS restores each mutated file by renaming its backup from here,
  // so the folder sits beside the sources: `target/` may be another volume
  // (a slot worktree's build folder links one), where the rename fails and
  // leaves the mutant in the source.
  tempDirName: ".stryker-tmp",
  thresholds: { high: 80, low: 60, break: null },
  reporters: ["clear-text", "progress", "html", "json"],
  htmlReporter: { fileName: `${reports}/report.html` },
  jsonReporter: { fileName: `${reports}/report.json` },
};

export default config;
