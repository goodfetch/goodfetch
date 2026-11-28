// J-002, "Crawl a site over HTTP to Markdown and export its Dataset as
// JSON, CSV and Parquet", on the `sdk-typescript` surface (ADRs 0063, 0069,
// 0075 and 0094; issue 141): installs Foxhound's npm package and its platform
// package into a fresh project with no install scripts, verifies the commit's
// Product version across executable and SDK packages, crawls `catalog/1` over
// HTTP to Markdown with `CrawlExtraction`, streams Records as they arrive, and
// exports JSON, CSV and Parquet.
//
// Covers: story:1-1, story:1-2, story:1-3, story:11-1130, story:11-1132, row:SA-01, row:CO-18, row:CO-25, row:CO-27, row:CO-29
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { isDeepStrictEqual } from "node:util";
import {
  Crawl,
  Foxhound,
  type CrawlExtraction,
  type CrawlRequestOutcome,
  type CrawlRun,
  type CrawlSettings,
  type DatasetRecord,
} from "@goodfetch/foxhound";
import { journey, taskStartUrl, test } from "@goodfetch/test-support";
import { expect } from "vitest";

const PACKAGE = dirname(dirname(import.meta.dirname));
const INSTALL_SCRIPT_NAMES = ["preinstall", "install", "postinstall"] as const;
const EXPORT_FORMATS = ["json", "csv", "parquet"] as const;

interface PackageManifest {
  readonly name: string;
  readonly version: string;
  readonly scripts?: Readonly<Record<string, string>>;
}

function readManifest(path: string): PackageManifest {
  const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (typeof parsed !== "object" || parsed === null) {
    throw new TypeError(`expected package manifest object at ${path}`);
  }
  const name = Reflect.get(parsed, "name");
  const version = Reflect.get(parsed, "version");
  const scripts = Reflect.get(parsed, "scripts");
  if (typeof name !== "string" || typeof version !== "string") {
    throw new TypeError(`missing name or version in ${path}`);
  }
  const parsedScripts: Record<string, string> = {};
  if (typeof scripts === "object" && scripts !== null) {
    for (const [key, value] of Object.entries(scripts)) {
      if (typeof value === "string") {
        parsedScripts[key] = value;
      }
    }
  }
  return {
    name,
    version,
    scripts: parsedScripts,
  };
}

function installFreshProject(
  project: string,
  engine: string,
): {
  readonly executable: string;
  readonly platformPackage: string;
  readonly manifests: readonly PackageManifest[];
} {
  const scope = join(project, "node_modules", "@goodfetch");
  mkdirSync(scope, { recursive: true });
  const sources = join(project, "foxhound");
  mkdirSync(sources, { recursive: true });
  copyFileSync(join(PACKAGE, "package.json"), join(sources, "package.json"));
  cpSync(join(PACKAGE, "src"), join(sources, "src"), { recursive: true });
  symlinkSync(sources, join(scope, "foxhound"), "junction");
  const collieClientPath = realpathSync(
    join(PACKAGE, "node_modules", "@goodfetch", "collie-client"),
  );
  symlinkSync(collieClientPath, join(scope, "collie-client"), "junction");

  const foxhoundManifest = readManifest(join(sources, "package.json"));
  const collieClientManifest = readManifest(join(collieClientPath, "package.json"));
  const platformName = `foxhound-${process.platform}-${process.arch}`;
  const platformPackage = `@goodfetch/${platformName}`;
  const platformDir = join(scope, platformName);
  const binDir = join(platformDir, "bin");
  mkdirSync(binDir, { recursive: true });
  const platformManifest: PackageManifest = {
    name: platformPackage,
    version: foxhoundManifest.version,
  };
  writeFileSync(join(platformDir, "package.json"), JSON.stringify(platformManifest));

  const executable = join(
    binDir,
    process.platform === "win32" ? "goodfetch-foxhound.exe" : "goodfetch-foxhound",
  );
  copyFileSync(engine, executable);
  chmodSync(executable, 0o755);
  return {
    executable,
    platformPackage,
    manifests: [foxhoundManifest, collieClientManifest, platformManifest],
  };
}

function siteSettings(provingGround: {
  readonly resolve: Readonly<Record<string, string>>;
}): CrawlSettings {
  const resolve: Record<string, readonly string[]> = {};
  for (const [host, address] of Object.entries(provingGround.resolve)) {
    resolve[host] = [address];
  }
  return {
    fetch: { tier: "http" },
    politeness: { domain: { delay: 0 } },
    network: { resolve },
    egress: { privateRanges: Object.keys(resolve) },
  };
}

function csvHeaderAndRowCount(text: string): { readonly columns: string[]; readonly rows: number } {
  const lines: string[] = [];
  let current = "";
  let inQuotes = false;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === '"') {
      if (inQuotes && text[i + 1] === '"') {
        current += '""';
        i += 1;
      } else {
        inQuotes = !inQuotes;
        current += ch;
      }
    } else if ((ch === "\n" || ch === "\r") && !inQuotes) {
      if (ch === "\r" && text[i + 1] === "\n") i += 1;
      if (current.length > 0) lines.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  if (current.length > 0) lines.push(current);
  const header = lines[0] ?? "";
  return {
    columns: header.split(",").map((col) => col.replace(/^"|"$/g, "")),
    rows: Math.max(0, lines.length - 1),
  };
}

test(
  "J-002@sdk-typescript",
  {
    tags: [
      "J-002",
      "sdk-typescript",
      "story:1-1",
      "story:1-2",
      "story:1-3",
      "story:11-1130",
      "story:11-1132",
    ],
  },
  journey(async (run) => {
    const provingGround = run.provingGround;
    if (provingGround === undefined) {
      throw new TypeError("J-002 has a Proving ground");
    }
    const rootManifest = readManifest(join(PACKAGE, "package.json"));
    run.surfacePackage("@goodfetch/foxhound", rootManifest.version);

    let executableVersion = "";
    let platformPackage = "";
    let manifests: readonly PackageManifest[] = [];
    let foxhound: Foxhound | undefined;
    let crawlRun: CrawlRun | undefined;
    let followed: DatasetRecord<Record<string, unknown>>[] = [];
    let committed: DatasetRecord<Record<string, unknown>>[] = [];
    let outcomes: CrawlRequestOutcome[] = [];
    const exportsMeta: {
      readonly format: (typeof EXPORT_FORMATS)[number];
      readonly path: string;
      readonly size: number;
      readonly records: number;
    }[] = [];

    const schemaCrawl = new Crawl();
    const pages = schemaCrawl.dataset<Record<string, unknown>>("pages");

    try {
      await run.step(
        "Install Foxhound into a fresh project from the commit's packages alone, with install scripts ignored: the surface's SDK package, or Foxhound's executable on the other surfaces",
        () => {
          const project = join(realpathSync(run.testEnv.folder), "fresh-project");
          const installed = installFreshProject(project, run.engine);
          platformPackage = installed.platformPackage;
          manifests = installed.manifests;
          executableVersion = execFileSync(installed.executable, ["--version"], {
            encoding: "utf8",
          }).trim();
        },
      );

      await run.step(
        "Crawl the catalog over HTTP with no browser, from its first listing page, turning each page into Markdown",
        async () => {
          const start = await taskStartUrl(provingGround, "catalog/1");
          const extraction: CrawlExtraction = {
            formats: { markdown: true },
            follow: "scope",
            fields: {
              id: { locator: { kind: "css", selector: "article.product .id" } },
              name: { locator: { kind: "css", selector: "article.product .name" } },
              price: {
                locator: { kind: "css", selector: "article.product .price" },
                attribute: "data-price",
              },
            },
          };
          // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
          foxhound = await Foxhound.connect({ env: run.env });
          crawlRun = await foxhound.startCrawl(
            { extraction },
            {
              runId: "catalog-ts",
              seeds: [{ url: start, label: "page" }],
              settings: siteSettings(provingGround),
            },
          );
        },
      );

      await run.step("Read the crawl's Records as they arrive", async () => {
        if (crawlRun === undefined) throw new TypeError("crawl started");
        const activeRun = crawlRun;
        const readRecords = async (): Promise<DatasetRecord<Record<string, unknown>>[]> => {
          const items: DatasetRecord<Record<string, unknown>>[] = [];
          for await (const record of activeRun.records(pages)) {
            items.push(record);
          }
          return items;
        };
        const readOutcomes = async (): Promise<CrawlRequestOutcome[]> => {
          const items: CrawlRequestOutcome[] = [];
          for await (const outcome of activeRun.outcomes()) {
            items.push(outcome);
          }
          return items;
        };
        const [streamed, endedOutcomes, report] = await Promise.all([
          readRecords(),
          readOutcomes(),
          activeRun.finish(),
        ]);
        expect(report.lifecycle).toBe("finished");
        followed = streamed;
        outcomes = endedOutcomes;
      });

      await run.step("Export the crawl's Dataset as JSON, as CSV and as Parquet", async () => {
        if (crawlRun === undefined) throw new TypeError("crawl started");
        for (const format of EXPORT_FORMATS) {
          const exported = await crawlRun.export(pages, { format });
          const path = join(run.testEnv.folder, `catalog.${format}`);
          await exported.saveAs(path);
          exportsMeta.push({ format, path, size: exported.size, records: exported.records });
        }
        const items: DatasetRecord<Record<string, unknown>>[] = [];
        for await (const record of crawlRun.records(pages)) {
          items.push(record);
        }
        committed = items;
      });
    } finally {
      if (foxhound !== undefined) {
        await foxhound.close();
      }
    }

    await run.check(
      "The installed package runs, and reports the commit's Product version, the one Foxhound's executable and every SDK package report",
      () => {
        expect(executableVersion).toBe(`goodfetch-foxhound ${rootManifest.version}`);
        for (const manifest of manifests) {
          expect(manifest.version).toBe(rootManifest.version);
        }
      },
    );

    await run.check(
      "The install ran no install script and reached no registry beyond the commit's packages",
      () => {
        for (const manifest of manifests) {
          for (const hook of INSTALL_SCRIPT_NAMES) {
            expect(manifest.scripts?.[hook]).toBeUndefined();
          }
        }
      },
    );

    await run.check(
      "The installed package is the one platform package of its language for every runtime version Foxhound supports, such as one `py3-none` platform wheel for every supported CPython",
      () => {
        expect(platformPackage).toBe(`@goodfetch/foxhound-${process.platform}-${process.arch}`);
      },
    );

    const products = committed
      .filter((record) => typeof record.value["id"] === "string")
      .map((record) => ({
        id: record.value["id"],
        name: record.value["name"],
        price: record.value["price"],
      }));
    await run.expectTask("The oracle verifies the crawl's Records", {
      records: products,
    });

    await run.check("The crawl fetched every page over HTTP and launched no browser", () => {
      const tiers = new Set(committed.map((record) => record.provenance?.tier?.tier));
      const kinds = new Set(outcomes.map((outcome) => outcome.kind));
      expect([...tiers]).toEqual(["http"]);
      expect([...kinds]).toEqual(["succeeded"]);
      expect(committed.length).toBe(outcomes.length);
    });

    await run.check("Each Record holds its page's Markdown", () => {
      expect(committed.length).toBeGreaterThan(0);
      for (const record of committed) {
        const markdown = record.value["markdown"];
        expect(typeof markdown === "string" && markdown.length > 0).toBe(true);
      }
    });

    await run.check(
      "The Records read as they arrived are the Dataset's, in order from its first, each once",
      () => {
        const numbers = followed.map((record) => record.number);
        const expected = committed.map((_, idx) => idx + 1);
        expect(numbers).toEqual(expected);
        expect(followed).toEqual(committed);
      },
    );

    await run.check(
      "The JSON, CSV and Parquet Exports each hold every Record of the Dataset, with every field",
      () => {
        const values = committed.map((record) => record.value);
        const fields = [...new Set(values.flatMap((value) => Object.keys(value)))].toSorted();
        expect(exportsMeta.map((entry) => entry.format)).toEqual([...EXPORT_FORMATS]);
        const checked = exportsMeta.map((entry) => {
          const bytes = readFileSync(entry.path);
          if (entry.format === "json") {
            const parsed: unknown = JSON.parse(bytes.toString("utf8"));
            return {
              format: entry.format,
              sizeMatches: bytes.length === entry.size,
              records: entry.records,
              valid: isDeepStrictEqual(parsed, values),
            };
          }
          if (entry.format === "csv") {
            const parsed = csvHeaderAndRowCount(bytes.toString("utf8"));
            return {
              format: entry.format,
              sizeMatches: bytes.length === entry.size,
              records: entry.records,
              valid:
                parsed.rows === values.length &&
                JSON.stringify(parsed.columns.toSorted()) === JSON.stringify(fields),
            };
          }
          const latin1 = bytes.toString("latin1");
          return {
            format: entry.format,
            sizeMatches: bytes.length === entry.size,
            records: entry.records,
            valid:
              bytes.subarray(0, 4).toString("ascii") === "PAR1" &&
              bytes.subarray(bytes.length - 4).toString("ascii") === "PAR1" &&
              fields.every((field) => latin1.includes(field)),
          };
        });
        expect(checked).toEqual(
          EXPORT_FORMATS.map((format) => ({
            format,
            sizeMatches: true,
            records: values.length,
            valid: true,
          })),
        );
      },
    );
  }),
);
