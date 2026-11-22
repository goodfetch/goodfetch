// Foxhound's handled crawl through its TypeScript SDK, for `just compare`'s
// handled scenario (#2317): the program a Foxhound user writes, run by the
// built executable `GOODFETCH_FOXHOUND_ENGINE_PATH` names, which the SDK
// starts as its Attached engine. From `--url`, under the Label `page`, each
// page's handler reads its twenty fields in one `page.read` (ADR 0147),
// pushes them as one Record into the Dataset `--dataset` and enqueues the
// links of its nav, its children's list and its home link, under `page`
// again: four exchanges a page with the handler call. `--settings` are the
// Run's settings, as xtask gives the code-free arm's, with no `fetch.tier`,
// so the engine chooses each page's tier (#1602). It prints one line,
// `pages=<n>`, the handler calls that ended, once the Run has finished
// complete; xtask reads the Records from the Run's file afterwards.
//
//   node crawl.ts --url http://scale.test/sites/1000/1 --settings '{…}' \
//     --run compare --dataset pages
import process from "node:process";
import { parseArgs } from "node:util";

import { Crawl, Foxhound, seed, type PageField } from "@goodfetch/foxhound";

const { values } = parseArgs({
  options: {
    url: { type: "string" },
    settings: { type: "string" },
    run: { type: "string" },
    dataset: { type: "string" },
  },
  strict: true,
});
const { url, settings, run: runId, dataset: datasetName } = values;
if (
  url === undefined ||
  settings === undefined ||
  runId === undefined ||
  datasetName === undefined
) {
  process.stderr.write(
    "usage: node crawl.ts --url <url> --settings <json> --run <run id> --dataset <name>\n",
  );
  process.exit(2);
}

/** Whether `value` is a JSON object, as `--settings` is. */
const isObject = (value: unknown): value is Readonly<Record<string, unknown>> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const runSettings: unknown = JSON.parse(settings);
if (!isObject(runSettings)) {
  process.stderr.write("--settings takes a JSON object of the Run's settings\n");
  process.exit(2);
}

/** The element `selector` matches, read as its text, or its `attribute`. */
const css = (selector: string, attribute?: string): PageField => ({
  locator: { kind: "css", selector },
  ...(attribute === undefined ? {} : { attribute }),
});

const ITEM = "article.item";
const CHILD = (k: number): string => `ul.pages > li:nth-child(${k}) > a`;

/** A page of the article site's twenty fields. */
const FIELDS: Readonly<Record<string, PageField>> = {
  title: css("title"),
  name: css(`${ITEM} > h1.name`),
  id: css(ITEM, "data-id"),
  home: css("nav > a", "href"),
  home_label: css("nav > a"),
  header: css("header > p"),
  nav: css("nav > p"),
  footer: css("footer > p"),
  first_child: css(CHILD(1), "href"),
  first_child_name: css(CHILD(1)),
  second_child: css(CHILD(2), "href"),
  second_child_name: css(CHILD(2)),
  ...Object.fromEntries(
    Array.from({ length: 8 }, (_, index) => [
      `paragraph_${index + 1}`,
      css(`${ITEM} > p:nth-of-type(${index + 1})`),
    ]),
  ),
};

const crawl = new Crawl();
const page = crawl.label("page");
const pages = crawl.dataset<Readonly<Record<string, unknown>>>(datasetName);
let handled = 0;
crawl.handle(page, async (context) => {
  const { values: read } = await context.page.read(FIELDS);
  await context.push(pages, read);
  await context.enqueueLinks(context.page.locator("ul.pages a, nav > a"), page);
  handled += 1;
});

const foxhound = await Foxhound.connect();
try {
  const run = await foxhound.startCrawl(crawl, {
    seeds: [seed(url, page)],
    runId,
    settings: runSettings,
  });
  const report = await run.finish();
  if (report.lifecycle !== "finished" || report.completeness !== "complete") {
    throw new Error(`the crawl did not finish complete: ${JSON.stringify(report)}`);
  }
} finally {
  await foxhound.close();
}
process.stdout.write(`pages=${handled}\n`);
