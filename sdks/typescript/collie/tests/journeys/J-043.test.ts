// J-043, "Drive one page and see nothing leave the machine beyond its site
// and pinned downloads", on the `sdk-typescript` surface (ADRs 0043 and
// 0063): a program drives one product page of the catalog through the
// TypeScript SDK, on an Attached engine of Collie's executable whose Browser
// cache holds no browser and with nothing configured to leave the machine,
// so the first page downloads the pinned Chrome for Testing build from the
// Proving ground's mirror. The Proving ground's record then holds the page's
// requests to its site and that download, and nothing else. A connection to
// any other host would leave that record, which CI's outbound deny refuses
// and each test job's `check` step fails on (ADR 0043). Rust's
// `tests/it/journeys/j043.rs` and Python's `tests/journeys/test_j043.py`
// are its twins.
//
// Covers: story:4-80, story:4-81, row:SC-10
import { Collie, type Page, type Run } from "@goodfetch/collie";
import {
  journey,
  provingGroundObservations,
  taskStartUrl,
  test,
  type ProvingGroundObservations,
} from "@goodfetch/test-support";
import { expect } from "vitest";

/** The host the Proving ground serves the browser builds' mirror on. */
const BROWSER_BUILDS_HOST = "browser-builds.test";

/** The pinned path of a Chrome for Testing build's archive: `/<build>/<platform>/chrome-<platform>.zip`. */
const PINNED_ARCHIVE = /^\/\d+\.\d+\.\d+\.\d+\/(linux64|mac-arm64|mac-x64|win64)\/chrome-\1\.zip$/;

/** The parts of the connection record that list what reached the Proving ground other than its sites. */
const RECORD_PARTS = ["relays", "dns_queries", "udp_datagrams"] as const;

/** The counts of entries past the record's bound. */
const UNRECORDED = [
  "unrecorded_requests",
  "unrecorded_connections",
  "unrecorded_relays",
  "unrecorded_dns_queries",
  "unrecorded_udp_datagrams",
] as const;

/** The entries the observations list under `part` of the connection record. */
function entries(observations: ProvingGroundObservations, part: string): unknown[] {
  const listed: unknown = Reflect.get(observations, part);
  if (!Array.isArray(listed)) {
    throw new TypeError(`the observations list no ${part}`);
  }
  return listed;
}

/**
 * Everything the record holds beyond the page's requests to `site` and the
 * downloads from the mirror: each request to another host, each connection
 * a listener other than the sites' accepted, each relay, DNS query and UDP
 * datagram, and each entry past the record's bound.
 */
function beyondTheSite(observations: ProvingGroundObservations, site: string): unknown[] {
  return [
    ...observations.requests
      .filter((request) => request.host !== site && request.host !== BROWSER_BUILDS_HOST)
      .map((request) => ["request", request.host, request.target]),
    ...entries(observations, "connections")
      .filter((connection) => Reflect.get(Object(connection), "listener") !== "site")
      .map((connection) => ["connection", connection]),
    ...RECORD_PARTS.flatMap((part) => entries(observations, part).map((entry) => [part, entry])),
    ...UNRECORDED.filter((count) => Reflect.get(observations, count) !== 0).map((count) => [
      count,
      Reflect.get(observations, count),
    ]),
  ];
}

test(
  "J-043@sdk-typescript",
  { tags: ["J-043", "sdk-typescript"] },
  journey(
    async (run) => {
      const provingGround = run.provingGround;
      if (provingGround === undefined) {
        throw new TypeError("J-043 has a Proving ground");
      }
      const product = await taskStartUrl(provingGround, "catalog/2");
      const port = provingGround.address.slice(provingGround.address.lastIndexOf(":") + 1);
      let collie: Collie | undefined;
      try {
        const started = await run.step(
          "Start a run with nothing configured to leave the machine, on a Browser cache that holds no browser",
          async (): Promise<Run> => {
            // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
            collie = await Collie.connect({ env: run.env });
            return collie.start();
          },
        );
        const page = await run.step(
          "Open a page, whose browser is downloaded on first use",
          async (): Promise<Page> => {
            if (started === undefined) {
              throw new TypeError("the run started");
            }
            return started.newPage();
          },
        );
        await run.step("Go to a product page of the site", async () => {
          await page?.goto(product);
        });
        const read = await run.step(
          "Read the product's ID, name and price from the page",
          async () =>
            page?.read({
              id: { locator: { kind: "css", selector: "span.id" } },
              name: { locator: { kind: "css", selector: "h1.name" } },
              price: {
                locator: { kind: "css", selector: "span.price" },
                attribute: "data-price",
                transform: { kind: "number", locale: "en" },
              },
            }),
        );
        const observations = await run.step("Finish the run", async () => {
          await started?.finish();
          return provingGroundObservations(provingGround);
        });

        await run.expectTask("The oracle verifies the product the page's fields answer", {
          records: read === undefined ? [] : [read.values],
        });
        await run.check(
          "The browser build was downloaded as one plain GET of its pinned path, with no cookie, no query and no header but its host and the user agent `goodfetch-download`",
          () => {
            const downloads = (observations?.requests ?? []).filter(
              (request) => request.host === BROWSER_BUILDS_HOST,
            );
            expect(
              downloads.map((request) => ({
                method: request.method,
                target: PINNED_ARCHIVE.test(request.target)
                  ? "<the pinned archive>"
                  : request.target,
                overTls: request.tls !== undefined && request.tls !== null,
                headers: request.headers.toSorted(([one], [other]) => one.localeCompare(other)),
                cookies: request.cookies,
              })),
            ).toEqual([
              {
                method: "GET",
                target: "<the pinned archive>",
                overTls: true,
                headers: [
                  ["host", `${BROWSER_BUILDS_HOST}:${port}`],
                  ["user-agent", "goodfetch-download"],
                ],
                cookies: [],
              },
            ]);
          },
        );
        await run.check(
          "Every request the Proving ground received went to the product page's host or was that download: no telemetry, no update check and no browser background traffic",
          () => {
            if (observations === undefined) {
              throw new TypeError("the run finished");
            }
            expect(beyondTheSite(observations, new URL(product).hostname)).toEqual([]);
          },
        );
      } finally {
        await collie?.close();
      }
    },
    { browserDownload: "served" },
  ),
);
