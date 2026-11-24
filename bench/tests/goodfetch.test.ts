// The Goodfetch adapter's smoke campaign (spec #152, issue 184): `bench
// run` drives Collie on the catalog's product page and Foxhound on its
// listing, each Product's executable of this commit through its own SDK,
// beside the stub competitor, against the commit's Proving ground. The run
// record names each adapter's Product and row, and the matched row's
// settings: the control the campaign switches off, and Filter-list
// blocking and Consent prompt handling as the stub's published
// configuration declares its defaults.
import { execFile } from "node:child_process";
import { mkdtemp, readFile, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { productEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import type { Attempt } from "../src/cell.ts";
import type { RunRecord } from "../src/runner.ts";

const exec = promisify(execFile);
const cli = new URL("../src/cli.ts", import.meta.url).pathname;
const campaign = new URL("campaigns/goodfetch.json", import.meta.url).pathname;

describe("the Goodfetch adapter", { tags: ["story:2-25"] }, () => {
  test.override("provingGroundSeed", 152);
  test.override("withBrowser", true);

  test("runs each Product on the family it carries, by default and matched, and the run record names its Product and marks its settings", async ({
    testEnv,
    provingGround,
  }) => {
    const out = await mkdtemp(join(tmpdir(), "goodfetch-bench-"));
    // The test's home and both Products' executables, as each SDK reads them.
    const env = {
      PATH: process.env["PATH"] ?? "",
      ...productEnv(testEnv, "collie"),
      ...productEnv(testEnv, "foxhound"),
    };
    const args = [cli, "run", campaign, "--proving-ground", provingGround.address, "--out", out];
    const { stderr } = await exec(process.execPath, args, { env });
    expect(stderr).toBe("");

    const [folder] = await readdir(out);
    const run = join(out, folder ?? "");
    const record: RunRecord = JSON.parse(await readFile(join(run, "run.json"), "utf8"));
    const attempts = (await readFile(join(run, "attempts.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map((line): Attempt => JSON.parse(line));

    expect(record.adapters).toEqual([
      { id: "stub", product: null, row: { kind: "default" }, settings: null },
      { id: "collie", product: "collie", row: { kind: "default" }, settings: {} },
      {
        id: "collie-matched",
        product: "collie",
        row: {
          kind: "matched",
          against: "stub",
          settings: { "live_view.hiding": "off" },
          competitor_defaults: { filter_list_blocking: false, consent_prompt_handling: false },
        },
        settings: {
          "live_view.hiding": "off",
          "filter_lists.ads": false,
          "filter_lists.trackers": false,
          "filter_lists.annoyances": false,
          "consent.handling": "off",
        },
      },
      { id: "foxhound", product: "foxhound", row: { kind: "default" }, settings: {} },
    ]);

    // Each Product is verified on the kind it carries, and unsupported on the other.
    const outcomes = Object.fromEntries(
      record.cells.map((cell) => [
        cell.cell_id,
        attempts
          .filter((attempt) => attempt.cell_id === cell.cell_id)
          .map((attempt) => attempt.status)
          .filter((status, index, all) => all.indexOf(status) === index),
      ]),
    );
    expect(outcomes).toEqual({
      "stub:catalog/1:b0": ["unsupported", "skipped_gate"],
      "stub:catalog/2:b0": ["verified"],
      "collie:catalog/1:b0": ["unsupported", "skipped_gate"],
      "collie:catalog/2:b0": ["verified"],
      "collie-matched:catalog/1:b0": ["unsupported", "skipped_gate"],
      "collie-matched:catalog/2:b0": ["verified"],
      "foxhound:catalog/1:b0": ["verified"],
      "foxhound:catalog/2:b0": ["unsupported", "skipped_gate"],
    });
  });
});
