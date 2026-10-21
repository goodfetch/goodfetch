import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, onTestFinished, test } from "vitest";
import type { JourneyFacts } from "./journey-document.ts";
import JourneyReporter, { JOURNEY_FACTS } from "./journey-reporter.ts";
import { journeyVectors } from "./outcome-vectors.ts";

const variant = {
  journey_id: "J-001",
  surface: "console",
  executable: { product: "collie", version: "0.0.0" },
  platform: { os: "linux", arch: "x86_64" },
  runtime: "chromium-141",
} as const;

/** A folder of the test's own, removed when it ends. */
function folder(): string {
  const made = mkdtempSync(join(tmpdir(), "journey-reporter-"));
  onTestFinished(() => rmSync(made, { recursive: true, force: true }));
  return made;
}

/** A test result carrying `facts` as the Journey helper attaches them. */
function resultOf(facts: JourneyFacts): {
  attachments: { name: string; contentType: string; body: Buffer }[];
} {
  return {
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

describe("JourneyReporter", () => {
  test.for(journeyVectors(variant))("writes the vector $name's outcome", ({ facts, outcome }) => {
    const out = folder();
    const reporter = new JourneyReporter({ out });

    reporter.onTestEnd(undefined, resultOf(facts));

    expect(written(out).outcome).toBe(outcome);
  });

  test("writes nothing for a test that carries no Journey's facts", () => {
    const out = folder();
    const reporter = new JourneyReporter({ out });

    reporter.onTestEnd(undefined, { attachments: [] });

    expect(readdirSync(out)).toEqual([]);
  });

  test("keeps a variant failed when another project's run of it passes after", () => {
    const out = folder();
    const reporter = new JourneyReporter({ out });

    reporter.onTestEnd(undefined, resultOf(vector("a-step-failed")));
    reporter.onTestEnd(undefined, resultOf(vector("every-step-check-and-task-passed")));

    expect(written(out).outcome).toBe("failed");
  });
});
