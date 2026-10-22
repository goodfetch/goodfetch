// Seam B (spec #326): `browserState` and `browserStateReset` on a project
// whose provider sets a colour scheme and reduced motion other than
// Playwright's defaults (ADR 0068). The fixture project of
// fixtures/browser-state/ runs in its own Vitest, so the values the reset
// must give back are the project's literal ones, never a reading taken in a
// shuffled file after another test's reset.
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect } from "vitest";
import { test } from "@goodfetch/test-support";

const vitest = join(
  dirname(createRequire(import.meta.url).resolve("vitest/package.json")),
  "vitest.mjs",
);
const fixture = fileURLToPath(new URL("fixtures/browser-state/", import.meta.url));

test("browserStateReset gives back the project's colour scheme and motion", async ({
  processes,
  onTestFinished,
}) => {
  const output = await mkdtemp(join(tmpdir(), "goodfetch-browser-state-"));
  onTestFinished(() => rm(output, { recursive: true, force: true }));
  const run = processes.spawn(
    process.execPath,
    [vitest, "run", "--config", "vitest.config.ts", "--root", "."],
    { env: { ...process.env, GOODFETCH_FIXTURE_OUTPUT: output }, cwd: fixture },
  );
  let stdout = "";
  run.child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
    stdout += chunk;
  });
  run.child.stderr.resume();

  await run.exit;

  expect(stdout.split("\n").filter((line) => line.startsWith("ran: "))).toEqual([
    "ran: colour scheme and motion passed",
  ]);
});
