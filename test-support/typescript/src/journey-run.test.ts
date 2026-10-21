import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, onTestFinished, test } from "vitest";
import { TestSupportError } from "./errors.ts";
import { readJourneyFile } from "./journey-file.ts";
import { JourneyRun } from "./journey-run.ts";

/** J-961: a Collie Journey with a `cli` step, a manual step and a console step, and one check. */
const J961 = join(import.meta.dirname, "..", "tests", "fixtures", "journeys", "J-961.toml");

const VERSION = "Ask the executable for its version";
const MANUAL = "See the system browser open the local Console";
const OPEN = "Open the local Console's address";
const NAMES = "The Console names the Daemon's Product";

function start(): JourneyRun {
  return new JourneyRun({
    journey: readJourneyFile(J961),
    surface: "console",
    executable: { product: "collie", version: "0.0.0" },
    platform: { os: "linux", arch: "x86_64" },
    runtime: "chromium-141",
    provingGround: undefined,
  });
}

/** Each recorded step or check as `<outcome> <text>`. */
function outcomes(recorded: readonly { text: string; outcome: string }[]): string[] {
  return recorded.map(({ text, outcome }) => `${outcome} ${text}`);
}

describe("JourneyRun", () => {
  test("records a complete run in the Journey's order, its manual step manual", async () => {
    const run = start();

    await run.step(VERSION, () => {});
    await run.step(OPEN, () => {});
    await run.check(NAMES, () => {});
    const facts = run.facts();

    expect(outcomes(facts.steps)).toEqual([
      `passed ${VERSION}`,
      `manual ${MANUAL}`,
      `passed ${OPEN}`,
    ]);
    expect(outcomes(facts.checks)).toEqual([`passed ${NAMES}`]);
    expect(run.failures).toEqual([]);
  });

  test("records a step and a check never called as not reached", async () => {
    const run = start();

    await run.step(VERSION, () => {});
    const facts = run.facts();

    expect(outcomes(facts.steps)).toEqual([
      `passed ${VERSION}`,
      `manual ${MANUAL}`,
      `not_reached ${OPEN}`,
    ]);
    expect(outcomes(facts.checks)).toEqual([`not_reached ${NAMES}`]);
  });

  test("records a skipped step after those called, out of the Journey's order", async () => {
    const run = start();

    await run.step(OPEN, () => {});
    const facts = run.facts();

    expect(outcomes(facts.steps)).toEqual([
      `failed ${OPEN}`,
      `not_reached ${VERSION}`,
      `manual ${MANUAL}`,
    ]);
  });

  test("records a step called out of order as failed without running it", async () => {
    const run = start();
    let ran = false;

    await run.step(OPEN, () => {
      ran = true;
    });

    expect(ran).toBe(false);
    expect(run.failures).toEqual([`step \`${OPEN}\` is called before step 1 \`${VERSION}\``]);
  });

  test("runs nothing after a failed step, and records the rest not reached", async () => {
    const run = start();
    let ran = false;

    await run.step(VERSION, () => {
      throw new Error("no version");
    });
    await run.step(OPEN, () => {
      ran = true;
    });
    await run.check(NAMES, () => {
      ran = true;
    });
    const facts = run.facts();

    expect(ran).toBe(false);
    expect(outcomes(facts.steps)).toEqual([
      `failed ${VERSION}`,
      `manual ${MANUAL}`,
      `not_reached ${OPEN}`,
    ]);
    expect(outcomes(facts.checks)).toEqual([`not_reached ${NAMES}`]);
  });

  test("records a step still running when the run ends as failed", async () => {
    const run = start();

    void run.step(VERSION, () => new Promise(() => {}));
    const facts = run.facts();

    expect(outcomes(facts.steps)[0]).toBe(`failed ${VERSION}`);
  });

  test("fails a task its Journey does not name, and records it with no answer", async () => {
    const run = start();

    const answer = await run.expectTask("catalog/1", { status: "success", records: [] });
    const facts = run.facts();

    expect(answer).toBeUndefined();
    expect(facts.tasks).toEqual([{ id: "catalog/1" }]);
    expect(run.failures).toEqual(["task `catalog/1` is not one J-961 names"]);
  });
});

describe("readJourneyFile", () => {
  test("refuses a file its schema refuses", () => {
    const folder = mkdtempSync(join(tmpdir(), "journey-file-"));
    onTestFinished(() => rmSync(folder, { recursive: true, force: true }));
    const file = join(folder, "J-961.toml");
    writeFileSync(file, 'id = "J-961"\ntitle = "No goal"\n');

    const read = (): unknown => readJourneyFile(file);

    expect(read).toThrow(TestSupportError);
    expect(read).toThrow(expect.objectContaining({ code: "journey_invalid" }));
  });
});
