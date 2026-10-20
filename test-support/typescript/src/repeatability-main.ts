// The repeatability measurement's command (ADR 0073), run by
// `ci/visual/run.sh measure` inside the pinned Playwright image on a copy of
// the repository (`just visual-repeatability`):
//
//   node test-support/typescript/src/repeatability-main.ts <runs> <browser,…>
//
// It measures every Playwright visual case of the working folder on each
// browser, prints each browser's verdict and each case's runs, writes them to
// target/visual/repeatability.json, and exits 1 when a browser's screenshots
// do not repeat.
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { ProcessRecord } from "./process-record.ts";
import { measureRepeatability } from "./repeatability.ts";
import { parseVisualBrowsers } from "./visual-config.ts";

const [runsArgument, browsersArgument] = process.argv.slice(2);
const runs = Number(runsArgument);
if (!Number.isInteger(runs) || runs < 1 || browsersArgument === undefined) {
  process.stderr.write("usage: repeatability-main.ts <runs> <browser,…>\n");
  process.exit(2);
}

const root = process.cwd();
const processes = new ProcessRecord();
const verdicts = await measureRepeatability({
  root,
  browsers: parseVisualBrowsers(browsersArgument),
  runs,
  environment: process.env,
  processes,
});
await processes.close();

const folder = join(root, "target", "visual");
mkdirSync(folder, { recursive: true });
writeFileSync(join(folder, "repeatability.json"), `${JSON.stringify(verdicts, null, 2)}\n`);
for (const verdict of verdicts) {
  const summary = verdict.repeats ? "repeats" : "differs";
  process.stdout.write(
    `${verdict.browser}: ${summary}, visual cases: ${verdict.cases.length}, runs of each: ${runs}\n`,
  );
  for (const runsOfCase of verdict.cases) {
    process.stdout.write(
      `  ${runsOfCase.file} › ${runsOfCase.title}: ${runsOfCase.repeated} of ${runsOfCase.runs} runs repeated\n`,
    );
  }
}
process.exitCode = verdicts.every((verdict) => verdict.repeats) ? 0 : 1;
