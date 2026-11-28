// J-031, "Crawl with robots.txt turned off and find it in the Run report,
// the Audit log and the Records' Provenance", on the `sdk-typescript` surface
// (ADRs 0063, 0075 and 0086; spec #565, ticket #688): runs a crawl through
// `@goodfetch/foxhound` over an allowed and a disallowed page of
// `1.access-policy.test` with `site_preferences.respect_robots_txt` off and
// `site_preferences.mark_robots_txt_ignored` on, reads the Run report, exports
// the Audit log, and checks the Records' Provenance.
//
// Covers: story:4-117, story:4-118, row:SC-01, row:compliance-site-owner-politeness
import {
  Crawl,
  Foxhound,
  seed,
  type CrawlRun,
  type CrawlRunReport,
  type CrawlSettings,
  type DatasetRecord,
} from "@goodfetch/foxhound";
import {
  journey,
  provingGroundUrl,
  test,
  type ProvingGroundDescription,
  type RecordedProcess,
} from "@goodfetch/test-support";
import { expect } from "vitest";

const HOST = "1.access-policy.test";
const RUN_ID = "robots-txt-off";

interface PageRecord {
  readonly url: string;
}

function settingsFor(provingGround: ProvingGroundDescription): CrawlSettings {
  const ip = provingGround.resolve[HOST];
  if (ip === undefined) {
    throw new TypeError(`Proving ground does not resolve ${HOST}`);
  }
  return {
    fetch: { tier: "http" },
    network: { resolve: { [HOST]: [ip] } },
    egress: { privateRanges: [HOST] },
    sitePreferences: {
      respectRobotsTxt: false,
      markRobotsTxtIgnored: true,
    },
  };
}

async function readOutput(proc: RecordedProcess): Promise<{
  readonly code: number | null;
  readonly stdout: string;
  readonly stderr: string;
}> {
  let stdout = "";
  let stderr = "";
  proc.child.stdout.setEncoding("utf8");
  proc.child.stderr.setEncoding("utf8");
  proc.child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  proc.child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });
  proc.child.stdin.end();
  const exit = await proc.exit;
  return { code: exit.code, stdout, stderr };
}

test(
  "J-031@sdk-typescript",
  {
    tags: [
      "J-031",
      "sdk-typescript",
      "story:4-117",
      "story:4-118",
      "row:SC-01",
      "row:compliance-site-owner-politeness",
    ],
  },
  journey(async (run) => {
    const provingGround = run.provingGround;
    if (provingGround === undefined) {
      throw new TypeError("J-031 has a Proving ground");
    }

    const crawl = new Crawl();
    const page = crawl.label("page");
    const pages = crawl.dataset<PageRecord>("pages");
    crawl.handle(page, async (context) => {
      await context.push(pages, { url: context.request.url });
    });

    let foxhound: Foxhound | undefined;
    let crawlRun: CrawlRun | undefined;
    let report: CrawlRunReport | undefined;
    let exported: readonly unknown[] = [];
    let records: DatasetRecord<PageRecord>[] = [];

    try {
      await run.step(
        "Run a crawl from a program over a page the site's robots.txt allows and one it disallows, with `site_preferences.respect_robots_txt` turned off and `site_preferences.mark_robots_txt_ignored` turned on for the run, each page's handler pushing a Record",
        async () => {
          // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
          foxhound = await Foxhound.connect({ env: run.env });
          crawlRun = await foxhound.startCrawl(crawl, {
            runId: RUN_ID,
            seeds: [
              seed(provingGroundUrl(provingGround, HOST, "/catalog/1"), page),
              seed(provingGroundUrl(provingGround, HOST, "/private/catalog/1"), page),
            ],
            settings: settingsFor(provingGround),
          });
          report = await crawlRun.finish();
        },
      );

      await run.step("Read the run's report", () => {
        expect(report?.lifecycle).toBe("finished");
      });

      await run.step("Export the Audit log", async () => {
        const result = await readOutput(run.command(["audit", "export"]));
        expect(result.code).toBe(0);
        exported = result.stdout
          .split(/\r?\n/)
          .filter((line) => line.length > 0)
          .map((line): unknown => JSON.parse(line));
      });

      await run.step("Read the crawl's Records", async () => {
        if (crawlRun === undefined) {
          throw new TypeError("crawl started");
        }
        const read: DatasetRecord<PageRecord>[] = [];
        for await (const record of crawlRun.records(pages)) {
          read.push(record);
        }
        records = read;
      });
    } finally {
      if (foxhound !== undefined) {
        await foxhound.close();
      }
    }

    await run.check(
      "The Run report lists `site_preferences.respect_robots_txt` in `weakened`, set by the run",
      () => {
        const listed = (report?.weakened ?? [])
          .filter((entry) => entry.key === "site_preferences.respect_robots_txt")
          .map((entry) => [entry.value, entry.layer]);
        expect(listed).toEqual([[false, "run"]]);
      },
    );

    await run.check(
      "The Audit log holds one `politeness_weakened` entry for the run, naming `site_preferences.respect_robots_txt`, its value `false`, its built-in default `true` and the run's layer",
      () => {
        const entries = exported
          .filter(
            (entry): entry is Record<string, unknown> =>
              typeof entry === "object" &&
              entry !== null &&
              Reflect.get(entry, "kind") === "politeness_weakened",
          )
          .map((entry) => [
            Reflect.get(entry, "run_id"),
            Reflect.get(entry, "layer"),
            Reflect.get(entry, "details"),
          ]);
        expect(entries).toEqual([
          [
            RUN_ID,
            "run",
            {
              setting: "site_preferences.respect_robots_txt",
              value: false,
              default: true,
            },
          ],
        ]);
      },
    );

    await run.check(
      'The Record of the disallowed page carries `robots_txt: "ignored"` in its Provenance, and the Record of the allowed page carries no mark',
      () => {
        const marks = records
          .map((record): readonly [string, string | null] => [
            new URL(record.value.url).pathname,
            record.provenance.robotsTxt ?? null,
          ])
          .toSorted(([a], [b]) => a.localeCompare(b));
        expect(marks).toEqual([
          ["/catalog/1", null],
          ["/private/catalog/1", "ignored"],
        ]);
      },
    );
  }),
);
