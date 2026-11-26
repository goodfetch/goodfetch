// The `bench` command line as the tests run it: one fixed campaign file of
// `campaigns/` against the test's Proving ground, writing under a new
// folder, and the run record and attempts that folder then holds.
import { execFile } from "node:child_process";
import { mkdtemp, readFile, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import type { Attempt } from "../src/cell.ts";
import type { RunRecord } from "../src/runner.ts";

const exec = promisify(execFile);
const cli = new URL("../src/cli.ts", import.meta.url).pathname;
const campaigns = new URL("campaigns/", import.meta.url).pathname;

/** How `bench run` exited, and the folder it wrote under. */
export interface Ran {
  readonly code: number;
  readonly stderr: string;
  readonly out: string;
}

/** Runs `bench run <campaign>` against `address`, writing under a new folder. */
export async function bench(campaign: string, address: string): Promise<Ran> {
  const out = await mkdtemp(join(tmpdir(), "goodfetch-bench-"));
  const env = { ...process.env, GOODFETCH_BENCH_STUB_STATE: out };
  const args = [cli, "run", join(campaigns, campaign), "--proving-ground", address, "--out", out];
  try {
    const { stderr } = await exec(process.execPath, args, { env });
    return { code: 0, stderr, out };
  } catch (error) {
    if (error instanceof Error && "code" in error && typeof error.code === "number") {
      return { code: error.code, stderr: "stderr" in error ? String(error.stderr) : "", out };
    }
    throw error;
  }
}

/** The run record and attempts of the one run `out` holds. */
export async function written(out: string): Promise<{ record: RunRecord; attempts: Attempt[] }> {
  const [run] = await readdir(out, { withFileTypes: true }).then((entries) =>
    entries.filter((entry) => entry.isDirectory()).map((entry) => join(out, entry.name)),
  );
  if (run === undefined) {
    throw new Error(`no run folder in ${out}`);
  }
  const record: RunRecord = JSON.parse(await readFile(join(run, "run.json"), "utf8"));
  const lines = (await readFile(join(run, "attempts.jsonl"), "utf8")).trim().split("\n");
  return { record, attempts: lines.map((line): Attempt => JSON.parse(line)) };
}
