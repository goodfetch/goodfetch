// Dataset Exports through `@goodfetch/foxhound` on an Attached engine
// (spec #611's #1488; ADRs 0103, 0116 and 0119): `run.export(dataset, { format })`
// returns an `Export` handle once the file is whole; `saveAs(path)` writes it
// atomically, creating missing folders, replacing existing files, and refusing
// unwritable paths with `PathNotWritableError`; `stream()` yields the same bytes;
// CSV has no BOM by default and carries one under `export.csv.bom`.
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  Crawl,
  Foxhound,
  PathNotWritableError,
  seed,
  type CrawlSettings,
  type ExportFormat,
} from "@goodfetch/foxhound";
import { taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const FORMATS: readonly ExportFormat[] = [
  "json",
  "jsonl",
  "csv",
  "xlsx",
  "xml",
  "parquet",
  "html_table",
  "rss",
  "atom",
  "avro",
  "arrow_ipc",
  "markdown",
];

const UTF8_BOM = Buffer.from([0xef, 0xbb, 0xbf]);

function siteSettings(provingGround: {
  readonly resolve: Readonly<Record<string, string>>;
}): CrawlSettings {
  const resolve: Record<string, readonly string[]> = {};
  for (const [host, address] of Object.entries(provingGround.resolve)) {
    resolve[host] = [address];
  }
  return {
    fetch: { tier: "http" },
    network: { resolve },
    egress: { privateRanges: Object.keys(resolve) },
  };
}

describe("Dataset Exports on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 1488);
  test.override("withBrowser", false);

  test(
    "exports every format matching size, sha256, saveAs and stream, honours CSV BOM and UTF-8, and refuses unwritable paths",
    {
      tags: [
        "attached",
        "story:6-276",
        "story:6-277",
        "operation:run.export",
        "operation:export.save_as",
        "operation:export.stream",
      ],
    },
    async ({ env, provingGround }) => {
      const start = await taskStartUrl(provingGround, "extraction/7");
      const outDir = await mkdtemp(join(tmpdir(), "gf-export-ts-"));
      try {
        const crawl = new Crawl();
        const page = crawl.label("page");
        const notes = crawl.dataset("notes");
        crawl.handle(page, async (ctx) => {
          await ctx.push(notes, { title: "Café — 日本語", count: 1 });
        });

        // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
        const foxhound = await Foxhound.connect({ env });
        try {
          const run = await foxhound.startCrawl(crawl, {
            runId: "ts-export",
            seeds: [seed(start, page)],
            settings: siteSettings(provingGround),
          });
          const report = await run.finish();
          expect(report.lifecycle).toBe("finished");

          for (const format of FORMATS) {
            const exported = await run.export(notes, { format });
            expect(exported.format).toBe(format);
            expect(exported.records).toBe(1);
            expect(exported.lastNumber).toBe(1);

            const path = join(outDir, format, "nested", `notes.${format}`);
            const saved = await exported.saveAs(path);
            const fileBytes = await readFile(path);
            const chunks: Uint8Array[] = [];
            for await (const chunk of exported.stream()) {
              chunks.push(chunk);
            }
            const streamed = Buffer.concat(chunks);
            const sha256 = createHash("sha256").update(fileBytes).digest("hex");

            expect(exported.size).toBe(fileBytes.length);
            expect(exported.sha256).toBe(sha256);
            expect(saved.size).toBe(exported.size);
            expect(saved.sha256).toBe(exported.sha256);
            expect(streamed.equals(fileBytes)).toBe(true);
          }

          // Default CSV has no BOM and preserves UTF-8 non-ASCII text.
          const csvPath = join(outDir, "csv", "nested", "notes.csv");
          const csvDefaultBytes = await readFile(csvPath);
          expect(csvDefaultBytes.subarray(0, 3).equals(UTF8_BOM)).toBe(false);
          expect(csvDefaultBytes.toString("utf8")).toContain("Café — 日本語");

          // CSV with `export.csv.bom: true` replaces existing file with a BOM-prefixed UTF-8 file.
          const csvBom = await run.export(notes, {
            format: "csv",
            settings: { export: { csv: { bom: true } } },
          });
          await csvBom.saveAs(csvPath);
          const bomBytes = await readFile(csvPath);
          expect(bomBytes.subarray(0, 3).equals(UTF8_BOM)).toBe(true);
          expect(bomBytes.subarray(3).toString("utf8")).toContain("Café — 日本語");

          // Unwritable path (an existing directory) raises `PathNotWritableError`, not retryable, leaving no partial file.
          const dirTarget = join(outDir, "existing-dir");
          await mkdir(dirTarget, { recursive: true });
          const caught: unknown = await csvBom.saveAs(dirTarget).catch((error: unknown) => error);
          const retryable = caught instanceof PathNotWritableError ? caught.retryable : undefined;
          expect([caught instanceof PathNotWritableError, retryable]).toEqual([true, false]);
          const entries = await readdir(outDir);
          expect(entries.filter((name) => name.includes(".partial"))).toEqual([]);
        } finally {
          await foxhound.close();
        }
      } finally {
        await rm(outDir, { recursive: true, force: true });
      }
    },
  );
});
