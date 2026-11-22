// StrykerJS runs that reach the Node programs a test starts (ADR 0072).
//
// StrykerJS switches its mutants, and learns which tests cover each, through
// `globalThis.__stryker__` in the test runner's process alone. The SDK's
// sources are mutated in place, so a program a test starts imports the
// mutated code too; this gives it the run's state:
//
// - while a mutant runs, the program starts with it active, through the
//   variable StrykerJS's instrumented code reads when its namespace names
//   none;
// - while StrykerJS learns which tests cover each mutant, the program
//   preloads `mutation-child.ts`, which writes each mutant id it covers to a
//   file; once it ended, they count for the test that started it, or for
//   every test when none was running.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** Each mutant id covered, with how many times. */
export type Coverage = Record<string, number>;

/** What StrykerJS's runner and instrumented code share in `globalThis.__stryker__`. */
export interface StrykerNamespace {
  /** The mutant switched on, during a mutant's run. */
  activeMutant?: string | undefined;
  /** The running test's id, while StrykerJS learns which tests cover each mutant. */
  currentTestId?: string | undefined;
  /** The mutants covered outside any test, and by each test. */
  mutantCoverage?: { static: Coverage; perTest: Record<string, Coverage> } | undefined;
}

declare global {
  /** StrykerJS's namespace, present only in a StrykerJS run. */
  // oxlint-disable-next-line no-underscore-dangle -- StrykerJS's own name for it.
  var __stryker__: StrykerNamespace | undefined;
}

/** The variable that names a program's coverage file to `mutation-child.ts`. */
export const COVERAGE_VARIABLE = "STRYKER_CHILD_COVERAGE";

const ACTIVE_MUTANT_VARIABLE = "__STRYKER_ACTIVE_MUTANT__";
const PRELOAD = pathToFileURL(join(import.meta.dirname, "mutation-child.ts")).href;

/** A program's environment in the run, and what to do once it ended. */
export interface MutationChild {
  readonly env: Readonly<Record<string, string | undefined>>;
  /** Counts what the program covered; call it once, after the program ended. */
  ended(): void;
}

/** Gives a program `env` plus the StrykerJS run's state, if `namespace` is a run's. */
export function reachMutationChild(
  env: Readonly<Record<string, string | undefined>>,
  namespace: StrykerNamespace | undefined,
): MutationChild {
  if (namespace === undefined) {
    return { env, ended: () => {} };
  }
  const { activeMutant } = namespace;
  if (activeMutant !== undefined) {
    return { env: { ...env, [ACTIVE_MUTANT_VARIABLE]: activeMutant }, ended: () => {} };
  }
  const folder = mkdtempSync(join(tmpdir(), "goodfetch-stryker-child-"));
  // Written empty first, so a program that covers nothing, or never starts,
  // leaves a file to read.
  const file = join(folder, "covered");
  writeFileSync(file, "");
  const test = namespace.currentTestId;
  const options = [env["NODE_OPTIONS"], `--import=${PRELOAD}`].filter(Boolean).join(" ");
  return {
    env: { ...env, NODE_OPTIONS: options, [COVERAGE_VARIABLE]: file },
    ended: () => {
      const covered = readFileSync(file, "utf8").split("\n").filter(Boolean);
      rmSync(folder, { recursive: true, force: true });
      const coverage = (namespace.mutantCoverage ??= { static: {}, perTest: {} });
      const counts = test === undefined ? coverage.static : (coverage.perTest[test] ??= {});
      for (const id of covered) {
        counts[id] = (counts[id] ?? 0) + 1;
      }
    },
  };
}
