import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, onTestFinished, test } from "vitest";
import { redact, type SnapshotValues } from "./snapshot.ts";

/** The shared cases every language's `redact` passes. */
const CASES: { cases: { description: string; output: string; redacted: string }[] } = JSON.parse(
  readFileSync(
    join(import.meta.dirname, "..", "..", "fixtures", "snapshot-filter-cases.json"),
    "utf8",
  ),
);

/** One run's values, as an output would print them. */
interface Run {
  id: string;
  schedule: string;
  instant: string;
  duration: string;
  folder: string;
  port: number;
  productVersion: string;
  commit: string;
}

function output(run: Run): string {
  return [
    `run ${run.id} of Schedule ${run.schedule} started ${run.instant}, took ${run.duration}`,
    `wrote ${run.folder}/home/report.json`,
    `engine goodfetch-collie ${run.productVersion} (${run.commit})`,
    `proving ground at http://127.0.0.1:${run.port}/pages`,
  ].join("\n");
}

function values(run: Run): SnapshotValues {
  return { folder: run.folder, productVersion: run.productVersion, commit: run.commit };
}

const FIRST: Run = {
  id: "01JH8Z5Q3W4X5Y6Z7A8B9C0D1E",
  schedule: "01JH8Z5Q3W4X5Y6Z7A8B9C0D1F",
  instant: "2026-01-05T09:00:00Z",
  duration: "1.5s",
  folder: "/tmp/snapshot first é 3kq9Zx",
  port: 54_321,
  productVersion: "0.3.1",
  commit: "4f1c2e9a7b3d5f6e8a9b0c1d2e3f4a5b6c7d8e9f",
};

const SECOND: Run = {
  id: "01K0AB1C2D3E4F5G6H7J8K9M0N",
  schedule: "01K0AB1C2D3E4F5G6H7J8K9M0P",
  instant: "2026-03-29T01:59:59.123+01:00",
  duration: "250ms",
  folder: "/tmp/snapshot second é Q2w8Lp",
  port: 8080,
  productVersion: "0.4.0",
  commit: "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567",
};

describe("redact", () => {
  test("makes two runs of one output snapshot alike", () => {
    const first = redact(output(FIRST), values(FIRST));
    const second = redact(output(SECOND), values(SECOND));

    expect(first).toMatchInlineSnapshot(`
      "run [ULID] of Schedule [ULID] started [INSTANT], took [DURATION]
      wrote [FOLDER]/home/report.json
      engine goodfetch-collie [PRODUCT_VERSION] ([COMMIT])
      proving ground at http://127.0.0.1:[PORT]/pages"
    `);
    expect(second).toMatchInlineSnapshot(`
      "run [ULID] of Schedule [ULID] started [INSTANT], took [DURATION]
      wrote [FOLDER]/home/report.json
      engine goodfetch-collie [PRODUCT_VERSION] ([COMMIT])
      proving ground at http://127.0.0.1:[PORT]/pages"
    `);
  });

  test.each(CASES.cases)(
    "redacts the shared case: $description",
    ({ output: printed, redacted }) => {
      expect(redact(printed, {})).toBe(redacted);
    },
  );

  test.skipIf(process.platform === "win32")(
    "redacts the test's folder in each form an output prints it",
    () => {
      const parent = mkdtempSync(join(tmpdir(), "snapshot é "));
      onTestFinished(() => rmSync(parent, { recursive: true, force: true }));
      const real = join(parent, 'real "q" \\b');
      mkdirSync(real);
      const folder = join(parent, "link");
      symlinkSync(real, folder);
      const canonical = realpathSync(folder);
      const printed = [
        `${folder}/a ${canonical}/b`,
        JSON.stringify({ path: `${folder}/c` }),
        JSON.stringify({ path: `${canonical}/d` }),
      ].join(" ");

      expect(redact(printed, { folder })).toBe(
        '[FOLDER]/a [FOLDER]/b {"path":"[FOLDER]/c"} {"path":"[FOLDER]/d"}',
      );
    },
  );

  test("leaves a value outside the shared rules in the snapshot", () => {
    const redacted = redact(
      "proxy https://example.com:8443/ answered token 7c1f in state succeeded",
      { folder: FIRST.folder },
    );

    expect(redacted).toMatchInlineSnapshot(
      `"proxy https://example.com:8443/ answered token 7c1f in state succeeded"`,
    );
  });
});
