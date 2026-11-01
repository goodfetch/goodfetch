// Seam B (spec #326): the `component-text-spacing` project's setup checks
// every test's page for clipped text once the test ends, so a screen whose
// label clips under WCAG's text spacing and 200% text fails without its test
// calling `clippedText` (ADR 0073). The fixture project of
// fixtures/text-spacing/ runs in its own Vitest.
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
const fixture = fileURLToPath(new URL("fixtures/text-spacing/", import.meta.url));

test("fails a test whose page clips text, and passes one whose page does not", async ({
  processes,
  onTestFinished,
}) => {
  const output = await mkdtemp(join(tmpdir(), "goodfetch-text-spacing-"));
  onTestFinished(() => rm(output, { recursive: true, force: true }));
  const run = processes.spawn(
    process.execPath,
    [vitest, "run", "--config", "vitest.config.ts", "--root", ".", "--sequence.shuffle=false"],
    { env: { ...process.env, GOODFETCH_FIXTURE_OUTPUT: output }, cwd: fixture },
  );
  let stdout = "";
  run.child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
    stdout += chunk;
  });
  run.child.stderr.resume();

  await run.exit;

  expect(stdout.split("\n").filter((line) => line.startsWith("ran: "))).toEqual([
    "ran: a label its fixed box cuts off under 200% text failed text_clipped",
    "ran: a label that wraps passed ",
  ]);
});
