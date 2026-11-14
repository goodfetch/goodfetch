// One trial of the Collie scenario's page-load cell (issue 2319): Collie's
// TypeScript SDK in a Session it launches itself, or Playwright in the same
// Chrome it launches, loads the page at `--url` to its `load` event and
// prints one JSON line of what the load came to.
//
// Collie's user configuration, which xtask wrote in the Goodfetch home it
// set, maps the page's host and its ad hosts to the Proving ground and turns
// off the defaults the row measures; Playwright's Chrome maps them with the
// `--resolve` rules, and with `--routed` every request passes through a
// route that continues it, as `page.route` does.
//
// Around the load it asks `--marks` for `/start` and `/end`, which xtask
// answers once it has read the process tree's CPU time. Launching the
// browser, opening the page and reading it stay outside the marks.
//
//   node load.ts --tool collie --url http://page-load.test:<port>/plain \
//     --marks http://127.0.0.1:<port>
//   node load.ts --tool playwright --url http://page-load.test:<port>/ads \
//     --marks http://127.0.0.1:<port> --executable <chrome> \
//     --resolve "MAP page-load.test 127.0.0.1, …" [--routed]
import process from "node:process";
import { parseArgs } from "node:util";

import { Collie } from "@goodfetch/collie";
import { chromium } from "playwright";

/** What the load came to. */
interface Loaded {
  /** The document's status, when the navigation answered one. */
  readonly status: number | null;
  /** The text of the page's `#content`, read after the end mark. */
  readonly read: string | null;
}

// An expression, which both tools evaluate as it is.
const READ = "document.querySelector('#content')?.textContent ?? null";

function readText(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

async function mark(marks: string, which: "start" | "end"): Promise<void> {
  const answer = await fetch(`${marks}/${which}`);
  if (!answer.ok) throw new Error(`the ${which} mark answered ${answer.status}`);
  await answer.arrayBuffer();
}

async function collie(url: string, marks: string): Promise<Loaded> {
  // The process's environment, which xtask set: the trial's Goodfetch home
  // and the engine's path.
  const connection = await Collie.connect();
  const run = await connection.start();
  const session = await run.newSession();
  const page = await session.newPage();
  await mark(marks, "start");
  const navigated = await page.goto(url, { waitUntil: "load" });
  await mark(marks, "end");
  const evaluated = await page.evaluate(READ);
  await page.close();
  await session.close();
  await run.finish();
  await connection.close();
  return { status: navigated.response?.status ?? null, read: readText(evaluated.value) };
}

async function playwright(
  url: string,
  marks: string,
  executable: string,
  resolve: string,
  routed: boolean,
): Promise<Loaded> {
  const browser = await chromium.launch({
    executablePath: executable,
    args: [`--host-resolver-rules=${resolve}`],
  });
  const context = await browser.newContext();
  if (routed) await context.route("**/*", (route) => route.continue());
  const page = await context.newPage();
  await mark(marks, "start");
  const response = await page.goto(url, { waitUntil: "load" });
  await mark(marks, "end");
  const read = readText(await page.evaluate<unknown>(READ));
  await context.close();
  await browser.close();
  return { status: response?.status() ?? null, read };
}

const { values } = parseArgs({
  options: {
    tool: { type: "string" },
    url: { type: "string" },
    marks: { type: "string" },
    executable: { type: "string" },
    resolve: { type: "string" },
    routed: { type: "boolean", default: false },
  },
  strict: true,
});
const usage =
  "usage: node load.ts --tool <collie|playwright> --url <page> --marks <http url> " +
  "[--executable <chrome> --resolve <rules> [--routed]]\n";
let loaded: Loaded;
if (values.url === undefined || values.marks === undefined) {
  process.stderr.write(usage);
  process.exit(2);
} else if (values.tool === "collie") {
  loaded = await collie(values.url, values.marks);
} else if (
  values.tool === "playwright" &&
  values.executable !== undefined &&
  values.resolve !== undefined
) {
  loaded = await playwright(
    values.url,
    values.marks,
    values.executable,
    values.resolve,
    values.routed,
  );
} else {
  process.stderr.write(usage);
  process.exit(2);
}
process.stdout.write(`${JSON.stringify(loaded)}\n`);
