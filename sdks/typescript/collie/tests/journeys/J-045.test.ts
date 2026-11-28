// J-045, "Develop against the Response cache, then test offline from a
// recording", on the `sdk-typescript` surface (ADRs 0046, 0063 and 0105;
// ticket #808): a developer runs an automation against a live Proving ground
// product page under `cache.mode: "replay"`, re-runs it against the same
// Response cache with zero requests to the Proving ground, records a Response
// recording, and runs it again offline with `recording: { mode: "replay" }`
// with zero requests and no miss listed in its Run report. Rust's
// `tests/it/journeys/j045.rs` and Python's `tests/journeys/test_j045.py` are
// its twins.
import { join } from "node:path";
import { Collie, type Recording, type RunReport, type RunSettings } from "@goodfetch/collie";
import { journey, provingGroundObservations, taskStartUrl, test } from "@goodfetch/test-support";
import { expect } from "vitest";

const PRODUCT_FIELDS = {
  id: { locator: { kind: "css" as const, selector: "span.id" } },
  name: { locator: { kind: "css" as const, selector: "h1.name" } },
  price: {
    locator: { kind: "css" as const, selector: "span.price" },
    attribute: "data-price",
    transform: { kind: "number" as const, locale: "en" },
  },
};

async function runProgram(
  collie: Collie,
  url: string,
  options: { settings?: RunSettings; recording?: Recording },
): Promise<{ values: Record<string, unknown>; report: RunReport }> {
  const started = await collie.start(options);
  const page = await started.newPage();
  await page.goto(url);
  const read = await page.read(PRODUCT_FIELDS);
  const report = await started.finish();
  return { values: read.values, report };
}

test(
  "J-045@sdk-typescript",
  { tags: ["J-045", "sdk-typescript"] },
  journey(async (run) => {
    const provingGround = run.provingGround;
    if (provingGround === undefined) {
      throw new TypeError("J-045 has a Proving ground");
    }
    const productUrl = await taskStartUrl(provingGround, "catalog/2");
    const warcPath = join(run.testEnv.folder, "product.warc");
    const resolve = Object.fromEntries(
      Object.entries(provingGround.resolve).map(([host, address]) => [host, [address]]),
    );
    const liveSite: RunSettings = {
      network: { resolve },
      egress: { privateRanges: Object.keys(provingGround.resolve) },
    };
    const requestCount = async (): Promise<number> =>
      (await provingGroundObservations(provingGround)).requests.length;

    let collie: Collie | undefined;
    try {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      collie = await Collie.connect({ env: run.env });
      const client = collie;

      let firstValues: Record<string, unknown> | undefined;
      let secondValues: Record<string, unknown> | undefined;
      let offlineValues: Record<string, unknown> | undefined;
      let offlineReport: RunReport | undefined;
      let requestsAfterFirst = 0;
      let requestsAfterSecond = 0;
      let requestsAfterRecord = 0;
      let requestsAfterOffline = 0;

      await run.step(
        "Run the program against a product page of the site under cache replay",
        async () => {
          const result = await runProgram(client, productUrl, {
            settings: { ...liveSite, cache: { mode: "replay" } },
          });
          firstValues = result.values;
          requestsAfterFirst = await requestCount();
        },
      );
      await run.step(
        "Run the same program a second time under cache replay on the same Response cache",
        async () => {
          const result = await runProgram(client, productUrl, {
            settings: { ...liveSite, cache: { mode: "replay" } },
          });
          secondValues = result.values;
          requestsAfterSecond = await requestCount();
        },
      );
      await run.step("Record a Response recording of the program against the site", async () => {
        await runProgram(client, productUrl, {
          settings: liveSite,
          recording: { mode: "record", path: warcPath },
        });
        requestsAfterRecord = await requestCount();
      });
      await run.step("Run the same program offline from the Response recording", async () => {
        const result = await runProgram(client, productUrl, {
          recording: { mode: "replay", path: warcPath },
        });
        offlineValues = result.values;
        offlineReport = result.report;
        requestsAfterOffline = await requestCount();
      });

      await run.expectTask("The oracle verifies the product the first run reads", {
        records: firstValues === undefined ? [] : [firstValues],
      });
      await run.expectTask(
        "The oracle verifies the product the second run reads from the Response cache",
        { records: secondValues === undefined ? [] : [secondValues] },
      );
      await run.check(
        "The second run under cache replay sends the Proving ground no request",
        () => {
          expect(requestsAfterFirst).toBeGreaterThan(0);
          expect(requestsAfterSecond).toBe(requestsAfterFirst);
        },
      );
      await run.expectTask(
        "The oracle verifies the product the offline run reads from the Response recording",
        { records: offlineValues === undefined ? [] : [offlineValues] },
      );
      await run.check(
        "The offline run from the Response recording sends the Proving ground no request and lists no miss in its Run report",
        () => {
          expect(requestsAfterRecord).toBeGreaterThan(requestsAfterSecond);
          expect(requestsAfterOffline).toBe(requestsAfterRecord);
          expect(offlineReport?.recording?.mode).toBe("replay");
          expect(offlineReport?.recording?.responsesUsed ?? 0).toBeGreaterThan(0);
          expect(offlineReport?.recording?.misses ?? []).toEqual([]);
        },
      );
    } finally {
      await collie?.close();
    }
  }),
);
