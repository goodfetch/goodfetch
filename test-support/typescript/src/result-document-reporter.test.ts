import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, onTestFinished, test } from "vitest";
import type { JourneyFacts } from "./journey-document.ts";
import { journeyVectors } from "./outcome-vectors.ts";
import ResultDocumentReporter, {
  CONFORMANCE_FACTS,
  JOURNEY_FACTS,
  type ReportedResult,
  type ReportedTest,
  type TargetFacts,
} from "./result-document-reporter.ts";

const variant = {
  journey_id: "J-001",
  surface: "console",
  executable: { product: "collie", version: "0.0.0" },
  platform: { os: "linux", arch: "x86_64" },
  runtime: "chromium-141",
} as const;

/** A folder of the test's own, removed when it ends. */
function folder(): string {
  const made = mkdtempSync(join(tmpdir(), "result-document-reporter-"));
  onTestFinished(() => rmSync(made, { recursive: true, force: true }));
  return made;
}

/** A test that runs no Journey variant and no conformance case. */
const plainTest: ReportedTest = {
  title: "a test",
  annotations: [],
  parent: { project: () => undefined },
};

/** A test's result carrying `facts` as the Journey helper attaches them; a passing one unless `failed` gives its errors. */
function resultOf(facts: JourneyFacts, failed?: readonly string[]): ReportedResult {
  return {
    status: failed === undefined ? "passed" : "failed",
    errors: (failed ?? []).map((message) => ({ message })),
    attachments: [
      {
        name: JOURNEY_FACTS,
        contentType: "application/json",
        body: Buffer.from(JSON.stringify(facts)),
      },
    ],
  };
}

/** The document the reporter wrote for J-001@console in `out`. */
function written(out: string): { outcome: string } {
  return JSON.parse(readFileSync(join(out, "J-001@console.json"), "utf8"));
}

/** The facts of the vector `name`. */
function vector(name: string): JourneyFacts {
  const found = journeyVectors(variant).find((one) => one.name === name);
  if (found === undefined) {
    throw new Error(`no outcome vector is named ${name}`);
  }
  return found.facts;
}

describe("ResultDocumentReporter's journey documents", () => {
  test.for(journeyVectors(variant))("writes the vector $name's outcome", ({ facts, outcome }) => {
    const out = folder();
    const reporter = new ResultDocumentReporter({ out });

    reporter.onTestEnd(plainTest, resultOf(facts));

    expect(written(out).outcome).toBe(outcome);
  });

  test("writes nothing for a test that carries no Journey's facts", async () => {
    const out = folder();
    const reporter = new ResultDocumentReporter({ out });

    reporter.onTestEnd(plainTest, { status: "passed", errors: [], attachments: [] });
    await reporter.onEnd();

    expect(readdirSync(out)).toEqual([]);
  });

  test("writes a variant failed when its test failed after every step and check passed", () => {
    const out = folder();
    const reporter = new ResultDocumentReporter({ out });

    reporter.onTestEnd(
      plainTest,
      resultOf(vector("every-step-check-and-task-passed"), [
        "TestSupportError: page_errors: console error: boom",
      ]),
    );

    expect(JSON.parse(readFileSync(join(out, "J-001@console.json"), "utf8"))).toMatchObject({
      outcome: "failed",
      test_errors: ["TestSupportError: page_errors: console error: boom"],
    });
  });

  test("keeps a variant failed when another project's run of it passes after", () => {
    const out = folder();
    const reporter = new ResultDocumentReporter({ out });

    reporter.onTestEnd(plainTest, resultOf(vector("a-step-failed")));
    reporter.onTestEnd(plainTest, resultOf(vector("every-step-check-and-task-passed")));

    expect(written(out).outcome).toBe("failed");
  });
});

const CASE_LIST = `
[suite]
name = "console"
version = "1"

[[capability]]
name = "runs.list"

[[capability.case]]
id = "runs-list-lists-every-run"
covers = []
`;

/** The facts a conformance test of the target attaches, on `seed`. */
function targetFacts(seed: number): TargetFacts {
  return {
    target: { product: "collie", version: "0.0.0" },
    platform: { os: "linux", arch: "x86_64" },
    runtime: "chromium-141",
    proving_ground: { version: "0.0.0+abc", seed },
    online: false,
  };
}

/** A test of the project naming the target `collie-daemon`, titled `title`, of the capability `runs.list`. */
function caseTest(title: string): ReportedTest {
  return {
    title,
    annotations: [{ type: "capability", description: "runs.list" }],
    parent: { project: () => ({ metadata: { conformanceTarget: "collie-daemon" } }) },
  };
}

/** A result of `status` carrying the target's facts on `seed`. */
function caseResult(status: ReportedResult["status"], seed = 1): ReportedResult {
  return {
    status,
    errors: [],
    attachments: [
      {
        name: CONFORMANCE_FACTS,
        contentType: "application/json",
        body: Buffer.from(JSON.stringify(targetFacts(seed))),
      },
    ],
  };
}

/** A reporter writing into a folder of the test's own, with `CASE_LIST` as its case list. */
function conformanceReporter(): { reporter: ResultDocumentReporter; out: string } {
  const out = folder();
  const caseList = join(out, "cases.toml");
  writeFileSync(caseList, CASE_LIST);
  return { reporter: new ResultDocumentReporter({ out, caseList }), out };
}

describe("ResultDocumentReporter's conformance documents", () => {
  test("leaves the case of a skipped test not reached", async () => {
    const { reporter, out } = conformanceReporter();

    reporter.onTestEnd(caseTest("runs-list-lists-every-run"), caseResult("skipped"));
    await reporter.onEnd();

    const document = JSON.parse(readFileSync(join(out, "collie-daemon.json"), "utf8"));
    expect(document.capabilities[0].cases[0].outcome).toBe("not_reached");
    expect(document.capabilities[0].outcome).toBe("unproved");
  });

  test("refuses a test whose title names no case of its capability, and fails the run", async () => {
    const { reporter } = conformanceReporter();

    expect(() => reporter.onTestEnd(caseTest("runs-list-is-sorted"), caseResult("passed"))).toThrow(
      expect.objectContaining({ code: "conformance_case_unknown" }),
    );
    expect(await reporter.onEnd()).toEqual({ status: "failed" });
  });

  test("refuses two tests of one target that report different Proving grounds", () => {
    const { reporter } = conformanceReporter();
    reporter.onTestEnd(caseTest("runs-list-lists-every-run"), caseResult("passed", 1));

    expect(() =>
      reporter.onTestEnd(caseTest("runs-list-lists-every-run"), caseResult("passed", 2)),
    ).toThrow(expect.objectContaining({ code: "conformance_target_mismatch" }));
  });

  test("writes nothing for a capability test of a project that names no target", async () => {
    const { reporter, out } = conformanceReporter();
    const untargeted = { ...caseTest("runs-list-lists-every-run"), parent: plainTest.parent };

    reporter.onTestEnd(untargeted, caseResult("passed"));
    await reporter.onEnd();

    expect(readdirSync(out)).toEqual(["cases.toml"]);
  });
});
