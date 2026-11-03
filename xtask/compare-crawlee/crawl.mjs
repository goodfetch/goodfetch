// One crawl with Crawlee's `CheerioCrawler`, for `just compare` (ADR 0138's
// matrix): from `--url`, at `--concurrency` requests at once, every page of
// its origin (`enqueueLinks`' default, the same hostname), with `--host` sent
// as each request's `Host` so the Proving ground serves its family on the
// loopback address. Without `--storage` nothing is kept on disk
// (`persistStorage: false`, scenario 1); with it, Crawlee keeps its storage
// there (`persistStorage: true`, its default) and does not purge it on start,
// so a process started again on the same folder resumes the crawl (scenario
// 2). It prints one line, `pages=<n>`, the pages Crawlee says it handled; the
// comparison's same-work check reads the Proving ground's own observations
// instead.
//
//   node crawl.mjs --url http://127.0.0.1:<port>/sites/1000/1 \
//     --host scale.test:<port> --concurrency 16 [--storage <folder>]

import process from "node:process";
import { parseArgs } from "node:util";

import { CheerioCrawler, Configuration } from "@crawlee/cheerio";

const { values } = parseArgs({
  options: {
    url: { type: "string" },
    host: { type: "string" },
    concurrency: { type: "string" },
    storage: { type: "string" },
  },
  strict: true,
});
const concurrency = Number(values.concurrency);
if (!values.url || !values.host || !Number.isInteger(concurrency) || concurrency < 1) {
  process.stderr.write(
    "usage: node crawl.mjs --url <url> --host <host:port> --concurrency <n> [--storage <folder>]\n",
  );
  process.exit(2);
}

const persist = values.storage !== undefined;
const configuration = new Configuration({
  persistStorage: persist,
  purgeOnStart: !persist,
  ...(persist ? { storageClientOptions: { localDataDirectory: values.storage } } : {}),
});

let pages = 0;
const crawler = new CheerioCrawler(
  {
    // At most 16 in flight, as the other tools. Its autoscaled pool starts
    // at the HTTP crawlers' desired concurrency, 10, and grows toward the
    // limit as the system allows: Crawlee's own behaviour, kept.
    maxConcurrency: concurrency,
    preNavigationHooks: [
      (_context, options) => {
        options.headers = { ...options.headers, host: values.host };
      },
    ],
    async requestHandler({ enqueueLinks }) {
      pages += 1;
      await enqueueLinks();
    },
  },
  configuration,
);

await crawler.run([values.url]);
process.stdout.write(`pages=${pages}\n`);
