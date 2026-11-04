// The harness of test support's own tests of its Playwright fixtures and
// reporter: Playwright runs the repository's own `playwright.config.ts`,
// copied unchanged into a folder of its own under `target/` with the Feature
// inventory it reads, on one spec file. The run writes its JUnit file under
// that folder's `target/` and its result documents into its own
// `result-documents/`, never into the outer run's, and the folder goes when
// the test ends.
import { copyFileSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { onTestFinished } from "vitest";
import type { ProcessRecord } from "@goodfetch/test-support";
import { runnerProject } from "./runner-project.ts";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const CLI = createRequire(import.meta.url).resolve("@playwright/test/cli");

/** What one run is given. */
export interface RepositoryPlaywright {
  /** The start of its folder's name under `target/`. */
  readonly folder: string;
  /** The spec file's name and text. */
  readonly spec: { readonly name: string; readonly text: string };
  /** Variables beside the outer run's, whose `GOODFETCH_TEST_SUITE` is `pre-merge`. */
  readonly env?: Readonly<Record<string, string>>;
  /** A configuration built on the copied one, such as one that sets an option in every project: its text, run in its place. */
  readonly config?: string;
  /** Playwright's `--workers`, when given. */
  readonly workers?: number;
  /** Report through Playwright's JSON reporter alone, into the run's `report.json`. */
  readonly json?: boolean;
}

/** How one run ended. */
export interface RepositoryPlaywrightRun {
  /** Its folder. */
  readonly project: string;
  /** Where its result documents went. */
  readonly documents: string;
  /** Playwright's exit code. */
  readonly code: number | null;
  /** The JSON report, when the run asked for one. */
  readonly report: string;
}

/** Runs Playwright on a copy of the repository's configuration, on the spec file `run.spec`. */
export async function runRepositoryPlaywright(
  processes: ProcessRecord,
  run: RepositoryPlaywright,
): Promise<RepositoryPlaywrightRun> {
  const project = runnerProject(run.folder);
  onTestFinished(() => rmSync(project, { recursive: true, force: true }));
  copyFileSync(join(ROOT, "playwright.config.ts"), join(project, "playwright.config.ts"));
  // The configuration reads the Feature inventory beside it.
  mkdirSync(join(project, "features"));
  copyFileSync(
    join(ROOT, "features", "inventory.json"),
    join(project, "features", "inventory.json"),
  );
  writeFileSync(join(project, run.spec.name), run.spec.text);
  let config = "playwright.config.ts";
  if (run.config !== undefined) {
    config = "built.config.ts";
    writeFileSync(join(project, config), run.config);
  }
  const documents = join(project, "result-documents");
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    GOODFETCH_TEST_SUITE: "pre-merge",
    GOODFETCH_TEST_RESULT_DOCUMENTS: documents,
    ...run.env,
  };
  delete env["PLAYWRIGHT_JUNIT_OUTPUT_FILE"];
  const report = join(project, "report.json");
  const reporter = run.json === true ? ["--reporter", "json"] : [];
  if (run.json === true) {
    env["PLAYWRIGHT_JSON_OUTPUT_FILE"] = report;
  }
  const workers = run.workers === undefined ? [] : ["--workers", String(run.workers)];
  const playwright = processes.spawn(
    process.execPath,
    [CLI, "test", "--config", config, ...workers, ...reporter],
    { env, cwd: project },
  );
  playwright.child.stdout.resume();
  playwright.child.stderr.resume();
  const { code } = await playwright.exit;
  return { project, documents, code, report };
}
