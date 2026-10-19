// Seam B (spec #326): the shared Vitest Browser Mode setup's `hoverReset`
// lets two shuffled tests of one file, one of them hovering, pass in either
// order; without it, the test that runs after the hovering one fails (ADRs
// 0064 and 0068). Each case runs the fixture project of fixtures/hover-reset/
// in its own Vitest, with an order seed that puts the hovering test first or
// second.
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";

const vitest = join(
  dirname(createRequire(import.meta.url).resolve("vitest/package.json")),
  "vitest.mjs",
);
const fixture = fileURLToPath(new URL("fixtures/hover-reset/", import.meta.url));

// Order seeds of the fixture's two tests under Vitest 5's shuffle.
const HOVERING_FIRST = "1";
const HOVERING_SECOND = "3";

interface Case {
  readonly name: string;
  readonly hoverReset: "on" | "off";
  readonly seed: string;
  readonly ran: readonly string[];
}

const cases: readonly Case[] = [
  {
    name: "hovering first, with hoverReset",
    hoverReset: "on",
    seed: HOVERING_FIRST,
    ran: ["ran: hovering passed", "ran: not hovered passed"],
  },
  {
    name: "hovering second, with hoverReset",
    hoverReset: "on",
    seed: HOVERING_SECOND,
    ran: ["ran: not hovered passed", "ran: hovering passed"],
  },
  {
    name: "hovering first, without hoverReset",
    hoverReset: "off",
    seed: HOVERING_FIRST,
    ran: ["ran: hovering passed", "ran: not hovered failed"],
  },
  {
    name: "hovering second, without hoverReset",
    hoverReset: "off",
    seed: HOVERING_SECOND,
    ran: ["ran: not hovered passed", "ran: hovering passed"],
  },
];

describe("hoverReset", () => {
  test.for(cases)("$name", async ({ hoverReset, seed, ran }, { processes, onTestFinished }) => {
    const output = await mkdtemp(join(tmpdir(), "goodfetch-hover-reset-"));
    onTestFinished(() => rm(output, { recursive: true, force: true }));
    const run = processes.spawn(
      process.execPath,
      [vitest, "run", "--config", "vitest.config.ts", "--root", ".", `--sequence.seed=${seed}`],
      {
        env: {
          ...process.env,
          GOODFETCH_FIXTURE_HOVER_RESET: hoverReset,
          GOODFETCH_FIXTURE_OUTPUT: output,
        },
        cwd: fixture,
      },
    );
    let stdout = "";
    run.child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
      stdout += chunk;
    });
    run.child.stderr.resume();

    await run.exit;

    expect(stdout.split("\n").filter((line) => line.startsWith("ran: "))).toEqual(ran);
  });
});
