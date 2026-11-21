// An edge function that asks for a crawl (ADR 0049): Vercel's edge runtime
// defines the global `EdgeRuntime`, which this program defines as its first
// act, as that runtime would, before it loads the SDK. It connects to the
// Attached engine its environment names, starts a crawl whose handler the
// engine would call after the request, and writes into the file its argument
// names the error that start failed with, then closes the Connection and
// exits 0. Run by Node alone, from the package's own source.
import { writeFileSync } from "node:fs";

declare global {
  /** Vercel's edge runtime's own global, which names it. */
  var EdgeRuntime: string | undefined;
}

globalThis.EdgeRuntime = "edge-runtime";
const { Crawl, Foxhound, isFoxhoundError, seed } = await import("@goodfetch/foxhound");

const [out] = process.argv.slice(2);
if (out === undefined) {
  throw new TypeError("the program takes the file it writes");
}

const env = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
);
// oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
const foxhound = await Foxhound.connect({ env });
const crawl = new Crawl();
const page = crawl.label("page");
crawl.handle(page, async () => {});

let failed: unknown;
try {
  await foxhound.startCrawl(crawl, { seeds: [seed("https://example.com/", page)] });
} catch (error) {
  failed = error;
}
await foxhound.close();

writeFileSync(
  out,
  JSON.stringify(
    isFoxhoundError(failed)
      ? {
          name: failed.name,
          code: failed.code,
          operation: failed.operation,
          message: failed.message,
          fix: failed.fix,
          details: failed.details,
        }
      : { failed: String(failed) },
  ),
);
