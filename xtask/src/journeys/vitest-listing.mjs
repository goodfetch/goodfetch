// Lists the workspace's Vitest tests with their tags, which `vitest list
// --json` leaves out, for `xtask journeys listings` (ADR 0063). It runs as
// `node --input-type=module --eval` from the workspace's folder, so `vitest`
// resolves from that folder's packages and Vitest reads its configuration
// there. It writes `[{ name, file, tags, mode }]` to the path it is given,
// `mode` being Vitest's: `run`, `only`, `skip` or `todo`.
import { writeFileSync } from "node:fs";
import { createVitest } from "vitest/node";

const out = process.argv[1];
// Vitest's own commands set these before they load the configuration, and
// `createVitest` does not: without `VITEST`, Storybook's addon gives no
// plugin, so the listing would not collect the stories a run collects.
process.env["TEST"] = "true";
process.env["VITEST"] = "true";
process.env["NODE_ENV"] ??= "test";
const vitest = await createVitest({ watch: false, run: true });
try {
  // A module that fails to collect, such as one with an undeclared tag,
  // sets the exit code, which fails the listing. `collect` runs no reporter,
  // so nothing would say why: each error goes to stderr, which the failure
  // quotes.
  const { testModules, unhandledErrors } = await vitest.collect();
  for (const testModule of testModules) {
    for (const error of testModule.errors()) {
      process.stderr.write(`${testModule.moduleId}: ${error.message}\n`);
    }
  }
  for (const error of unhandledErrors) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  }
  const tests = testModules.flatMap((testModule) =>
    [...testModule.children.allTests()].map((test) => ({
      name: test.fullName,
      file: testModule.moduleId,
      tags: test.tags,
      mode: test.options.mode,
    })),
  );
  writeFileSync(out, `${JSON.stringify(tests, null, 2)}\n`);
} finally {
  await vitest.close();
}
