// The Console's lab metrics for the Performance series (ADRs 0071 and 0076):
// Lighthouse 13 driven directly against each Product's Console, Collie's and
// Foxhound's, each served by its own executable (ADR 0087), signed in
// through the single-user link, five runs of the Runs screen, their medians
// of LCP, CLS and TBT held against the "good" values (LCP ≤ 2.5 s and CLS ≤
// 0.1 from Web Vitals, TBT ≤ 200 ms from web.dev's guidance). It times, so
// it belongs to no suite: `just console-lighthouse` runs it, and appends one
// record per Product, labelled with the Product and the machine it ran on,
// to the series' file, target/perf-series/console-lighthouse.jsonl. Only the
// owner's machine's records count for the series while there is no revenue;
// another machine's are labelled as such, never as the reference class.
import { execFileSync } from "node:child_process";
import { appendFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { arch, cpus, hostname, platform, release, tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "@playwright/test";
import {
  expect,
  reserveNamedPort,
  test,
  type ServedDaemon,
} from "@goodfetch/test-support/playwright";
import lighthouse from "lighthouse";

/** How many Lighthouse runs make one record, its figures their medians (ADR 0071). */
const RUNS = 5;

/** The "good" values each median is held against. */
const GOOD = { lcpMs: 2_500, cls: 0.1, tbtMs: 200 } as const;

/** Where the series' records go, from the repository's root. */
const SERIES = "target/perf-series";
const RECORDS = "console-lighthouse.jsonl";

/** One run's figures. */
interface Figures {
  readonly lcpMs: number;
  readonly cls: number;
  readonly tbtMs: number;
}

/** The median of `values`, which are not empty. */
function median(values: readonly number[]): number {
  const sorted = values.toSorted((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? ((sorted[middle - 1] ?? 0) + (sorted[middle] ?? 0)) / 2
    : (sorted[middle] ?? 0);
}

/** The commit measured, as Git names it. */
function commit(): string {
  return execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
}

/** Each Product whose Console build is measured, on its own executable (ADR 0087). */
const PRODUCTS = ["collie", "foxhound"] as const;

for (const product of PRODUCTS) {
  test.describe(`${product}'s Console`, () => {
    test.use({ product });

    test("its lab metrics on the Runs screen, the median of five Lighthouse runs", async ({
      daemon,
    }) => {
      await measure(product, daemon);
    });
  });
}

/** Measures the Console `daemon` serves, `product`'s, and appends its record to the series. */
async function measure(product: (typeof PRODUCTS)[number], daemon: ServedDaemon): Promise<void> {
  // The browser's DevTools endpoint, which Lighthouse drives, on a port no other program takes first.
  const named = await reserveNamedPort();
  const { port } = named;
  const profile = await mkdtemp(join(tmpdir(), "goodfetch-lighthouse-"));
  // The persistent context is the browser's default one, which the tab
  // Lighthouse opens shares, with the sign-in's cookie.
  const context = await chromium.launchPersistentContext(profile, {
    args: [`--remote-debugging-port=${String(port)}`],
  });
  try {
    const page = context.pages()[0] ?? (await context.newPage());
    await page.goto(await daemon.signInLink());
    await page.waitForURL((url) => url.hash === "");
    const runs = new URL("/runs", daemon.url).href;
    const measured: Figures[] = [];
    let version = "";
    for (let run = 0; run < RUNS; run += 1) {
      const result = await lighthouse(runs, {
        port,
        output: "json",
        logLevel: "error",
        onlyCategories: ["performance"],
        // Keeps the sign-in: Lighthouse otherwise clears the origin's storage.
        disableStorageReset: true,
      });
      version = result?.lhr.lighthouseVersion ?? version;
      const audits = result?.lhr.audits;
      const figure = (id: string): number => {
        const value = audits?.[id]?.numericValue;
        if (value === undefined) {
          throw new Error(
            `Lighthouse measured no ${id}: ${result?.lhr.runtimeError?.message ?? ""}`,
          );
        }
        return value;
      };
      measured.push({
        lcpMs: figure("largest-contentful-paint"),
        cls: figure("cumulative-layout-shift"),
        tbtMs: figure("total-blocking-time"),
      });
    }
    const medians: Figures = {
      lcpMs: median(measured.map(({ lcpMs }) => lcpMs)),
      cls: median(measured.map(({ cls }) => cls)),
      tbtMs: median(measured.map(({ tbtMs }) => tbtMs)),
    };
    const record = {
      at: new Date().toISOString(),
      commit: commit(),
      product,
      lighthouse: version,
      machine: {
        host: hostname(),
        os: `${platform()} ${release()}`,
        arch: arch(),
        cpu: cpus()[0]?.model ?? "unknown",
        label: process.env["GOODFETCH_PERF_MACHINE"] ?? "not the owner's machine",
      },
      runs: RUNS,
      medians,
      good: GOOD,
      within: {
        lcp: medians.lcpMs <= GOOD.lcpMs,
        cls: medians.cls <= GOOD.cls,
        tbt: medians.tbtMs <= GOOD.tbtMs,
      },
    };
    await mkdir(SERIES, { recursive: true });
    await appendFile(join(SERIES, RECORDS), `${JSON.stringify(record)}\n`);
    test.info().annotations.push({ type: "lighthouse", description: JSON.stringify(record) });
    expect(measured).toHaveLength(RUNS);
  } finally {
    await context.close();
    await rm(profile, { recursive: true, force: true });
    await named.release();
  }
}
