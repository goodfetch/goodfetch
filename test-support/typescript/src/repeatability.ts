// The repeatability measurement of ADR 0073: before a browser joins the
// `visual` projects, every Playwright visual case runs on it many times in
// the pinned Playwright image (ADR 0064's 20 runs), and the browser joins only
// if every run of every case gave the same pixels. `just visual-repeatability`
// runs it there (ci/visual/repeatability.sh); its result is recorded in the
// shared visual config's `REPEATABILITY`.
//
// For each browser, one Playwright run writes a fresh Reference screenshot of
// every case (`--update-snapshots=all`), then one run compares every case
// `runs` times against it at zero tolerance (`--repeat-each`). It therefore
// rewrites the References of the projects it measures: it runs on a copy of
// the repository, never on a checkout.
import { mkdirSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { Ajv2020, type SchemaObject, type ValidateFunction } from "ajv/dist/2020.js";
import { TestSupportError } from "./errors.ts";
import type { ProcessRecord } from "./process-record.ts";
import { visualProject, type Environment, type VisualBrowser } from "./visual-config.ts";

/** How many times one case ran on one browser, and how many of those runs repeated its pixels. */
export interface CaseRuns {
  /** The spec file, relative to the root. */
  readonly file: string;
  /** The case's title inside its file. */
  readonly title: string;
  readonly runs: number;
  readonly repeated: number;
}

/** What a measurement found on one browser. */
export interface BrowserVerdict {
  readonly browser: VisualBrowser;
  /** Every run of every case repeated, and there was at least one case. */
  readonly repeats: boolean;
  readonly cases: readonly CaseRuns[];
}

export interface RepeatabilityOptions {
  /** The folder of the `playwright.config.ts` whose visual cases are measured. */
  readonly root: string;
  readonly browsers: readonly VisualBrowser[];
  /** How many times each case runs against the Reference the first run wrote. */
  readonly runs: number;
  /** The environment Playwright runs in; GOODFETCH_VISUAL_IMAGE names the pinned image. */
  readonly environment: Environment;
  readonly processes: ProcessRecord;
}

/** Measures each of `browsers` in turn. */
export async function measureRepeatability(
  options: RepeatabilityOptions,
): Promise<readonly BrowserVerdict[]> {
  const verdicts: BrowserVerdict[] = [];
  for (const browser of options.browsers) {
    await playwright(options, browser, "update", ["--update-snapshots=all"]);
    const report = await playwright(options, browser, "runs", [`--repeat-each=${options.runs}`]);
    verdicts.push(browserVerdict(browser, caseRuns(report, visualProject(browser)), options.runs));
  }
  return verdicts;
}

/** The verdict of `browser` on its `cases`, each of which was asked to run `runs` times. */
export function browserVerdict(
  browser: VisualBrowser,
  cases: readonly CaseRuns[],
  runs: number,
): BrowserVerdict {
  return {
    browser,
    repeats: cases.length > 0 && cases.every((one) => one.runs === runs && one.repeated === runs),
    cases,
  };
}

/** The parts of Playwright's JSON report the measurement reads. */
interface Report {
  readonly suites: readonly Suite[];
}

interface Suite {
  readonly specs: readonly Spec[];
  readonly suites?: readonly Suite[];
}

interface Spec {
  readonly title: string;
  readonly file: string;
  /** One entry for each project and each repeat of the case. */
  readonly tests: readonly {
    readonly projectName: string;
    readonly status: "skipped" | "expected" | "unexpected" | "flaky";
  }[];
}

/** `Report`'s schema: a suite holds specs and suites. */
const REPORT_SCHEMA: SchemaObject = {
  type: "object",
  required: ["suites"],
  properties: { suites: { type: "array", items: { $ref: "#/$defs/suite" } } },
  $defs: {
    suite: {
      type: "object",
      required: ["specs"],
      properties: {
        specs: {
          type: "array",
          items: {
            type: "object",
            required: ["title", "file", "tests"],
            properties: {
              title: { type: "string" },
              file: { type: "string" },
              tests: {
                type: "array",
                items: {
                  type: "object",
                  required: ["projectName", "status"],
                  properties: {
                    projectName: { type: "string" },
                    status: { enum: ["skipped", "expected", "unexpected", "flaky"] },
                  },
                },
              },
            },
          },
        },
        suites: { type: "array", items: { $ref: "#/$defs/suite" } },
      },
    },
  },
};

const isReport: ValidateFunction<Report> = new Ajv2020().compile<Report>(REPORT_SCHEMA);

const CLI = createRequire(import.meta.url).resolve("@playwright/test/cli");

/** Runs Playwright on `browser`'s visual project and reads its JSON report. */
async function playwright(
  options: RepeatabilityOptions,
  browser: VisualBrowser,
  step: "update" | "runs",
  flags: readonly string[],
): Promise<Report> {
  const folder = join(options.root, "target", "visual", "repeatability");
  mkdirSync(folder, { recursive: true });
  const output = join(folder, `${browser}-${step}.json`);
  rmSync(output, { force: true });
  const run = options.processes.spawn(
    process.execPath,
    [CLI, "test", "--project", visualProject(browser), "--reporter=json", ...flags],
    {
      cwd: options.root,
      env: {
        ...options.environment,
        GOODFETCH_VISUAL_BROWSERS: browser,
        PLAYWRIGHT_JSON_OUTPUT_FILE: output,
      },
    },
  );
  run.child.stdout.resume();
  const errors: Buffer[] = [];
  run.child.stderr.on("data", (chunk: Buffer) => errors.push(chunk));
  await run.exit;
  let report: unknown;
  try {
    report = JSON.parse(readFileSync(output, "utf8"));
  } catch (error) {
    throw new TestSupportError(
      "repeatability_unmeasured",
      `Playwright's ${step} run on ${browser} wrote no readable report at ${output} (${String(error)}): ${Buffer.concat(errors).toString("utf8")}`,
    );
  }
  if (!isReport(report)) {
    throw new TestSupportError(
      "repeatability_unmeasured",
      `Playwright's ${step} run on ${browser} wrote a report the measurement cannot read: ${JSON.stringify(isReport.errors)}`,
    );
  }
  return report;
}

/** Each case's runs in `project`, in the report's order. */
function caseRuns(report: Report, project: string): CaseRuns[] {
  const cases: CaseRuns[] = [];
  const visit = (suite: Suite): void => {
    for (const spec of suite.specs) {
      const tests = spec.tests.filter((test) => test.projectName === project);
      if (tests.length > 0) {
        cases.push({
          file: spec.file,
          title: spec.title,
          runs: tests.length,
          repeated: tests.filter((test) => test.status === "expected").length,
        });
      }
    }
    for (const child of suite.suites ?? []) {
      visit(child);
    }
  };
  for (const suite of report.suites) {
    visit(suite);
  }
  return cases;
}
