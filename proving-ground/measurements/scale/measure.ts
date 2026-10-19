// The measurement `declaration.json` declares (ADR 0064): how fast the
// Proving ground serves the `scale` family's pages, and how fast Crawlee's
// HttpCrawler consumes them, on the same host. It is never a test: it runs by
// hand and writes its record, with the host it ran on, to `runs/`.
//
//   node proving-ground/measurements/scale/measure.ts \
//     --proving-ground target/release/goodfetch-proving-ground \
//     --crawlee <a node_modules folder holding crawlee 3.18.2>

import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import http from "node:http";
import { createRequire } from "node:module";
import os from "node:os";
import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

interface Serving {
  address: string;
  version: string;
  stop: () => Promise<void>;
}

interface Trial {
  kind: "proving_ground" | "crawlee";
  pages: number;
  seconds: number;
  pages_per_second: number;
}

interface Crawlee {
  HttpCrawler: new (
    options: object,
    config?: object,
  ) => { run: (requests: string[]) => Promise<unknown> };
  Configuration: new (options: object) => object;
}

function fail(message: string): never {
  throw new Error(message);
}

/** The member `key` of a parsed JSON value. */
function member(value: unknown, key: string): unknown {
  return typeof value === "object" && value !== null ? Reflect.get(value, key) : undefined;
}

function numberAt(value: unknown, key: string): number {
  const found = member(value, key);
  return typeof found === "number" ? found : fail(`${key} is not a number`);
}

function stringAt(value: unknown, key: string): string {
  const found = member(value, key);
  return typeof found === "string" ? found : fail(`${key} is not a string`);
}

function isCrawlee(value: unknown): value is Crawlee {
  return (
    typeof member(value, "HttpCrawler") === "function" &&
    typeof member(value, "Configuration") === "function"
  );
}

const { values } = parseArgs({
  options: {
    "proving-ground": { type: "string" },
    crawlee: { type: "string" },
  },
});
const usage = "usage: measure.ts --proving-ground <binary> --crawlee <node_modules folder>";
const binary = values["proving-ground"] ?? fail(usage);
const crawleeFrom = values.crawlee ?? fail(usage);

const here = new URL(".", import.meta.url);
const declared: unknown = JSON.parse(readFileSync(new URL("declaration.json", here), "utf8"));
const seed = numberAt(declared, "seed");
const familyPages = numberAt(declared, "pages");
const trialCount = numberAt(declared, "trials");
const competitorPages = numberAt(member(declared, "competitor"), "pages");
const host = "scale.test";
const connections = 64;
const seconds = 10;

/** A seeded draw of item numbers, the same for every trial (SplitMix64). */
function* items(): Generator<number, never, undefined> {
  let state = BigInt(seed);
  const mask = (1n << 64n) - 1n;
  for (;;) {
    state = (state + 0x9e3779b97f4a7c15n) & mask;
    let z = state;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & mask;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & mask;
    z ^= z >> 31n;
    yield Number(z % BigInt(familyPages)) + 1;
  }
}

async function serve(): Promise<Serving> {
  const child = spawn(
    binary,
    [
      "serve",
      "--seed",
      String(seed),
      "--copy",
      "local",
      "--address",
      "127.0.0.1:0",
      "--network-profile",
      "none",
    ],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout });
  const first = await new Promise<string>((resolve) => {
    lines.once("line", resolve);
  });
  const line: unknown = JSON.parse(first);
  return {
    address: stringAt(line, "address"),
    version: stringAt(line, "version"),
    stop: async () => {
      const exited = new Promise((resolve) => {
        child.once("exit", resolve);
      });
      child.kill("SIGTERM");
      await exited;
    },
  };
}

/** The pages per second the Proving ground answers `200` under the load. */
async function provingGroundTrial(serving: Serving): Promise<Trial> {
  const [hostname, port] = serving.address.split(":");
  const agent = new http.Agent({ keepAlive: true, maxSockets: connections });
  const draw = items();
  let pages = 0;
  const get = (item: number) =>
    new Promise<number>((resolve, reject) => {
      const request = http.get(
        { agent, hostname, port, path: `/items/${item}`, headers: { host: `${host}:${port}` } },
        (response) => {
          response.resume();
          response.once("end", () => {
            resolve(response.statusCode ?? 0);
          });
        },
      );
      request.once("error", reject);
    });
  const started = performance.now();
  const end = started + seconds * 1000;
  await Promise.all(
    Array.from({ length: connections }, async () => {
      while (performance.now() < end) {
        if ((await get(draw.next().value)) === 200) {
          pages += 1;
        }
      }
    }),
  );
  const elapsed = (performance.now() - started) / 1000;
  agent.destroy();
  return { kind: "proving_ground", pages, seconds: elapsed, pages_per_second: pages / elapsed };
}

/** The pages per second Crawlee's HttpCrawler consumes from the same draw. */
async function crawleeTrial(serving: Serving, crawlee: Crawlee): Promise<Trial> {
  const port = serving.address.split(":")[1];
  const draw = items();
  const urls = Array.from(
    { length: competitorPages },
    () => `http://${host}:${port}/items/${draw.next().value}`,
  );
  let pages = 0;
  const crawler = new crawlee.HttpCrawler(
    {
      minConcurrency: connections,
      maxConcurrency: connections,
      maxRequestRetries: 0,
      useSessionPool: false,
      requestHandler: () => {
        pages += 1;
      },
      // The `resolve` setting's mapping: the family's host name to loopback.
      preNavigationHooks: [
        (_context: unknown, options: { dnsLookup?: unknown }) => {
          options.dnsLookup = (
            _hostname: string,
            lookup: { all?: boolean },
            callback: (error: null, address: unknown, family?: number) => void,
          ) => {
            if (lookup.all === true) {
              callback(null, [{ address: "127.0.0.1", family: 4 }]);
            } else {
              callback(null, "127.0.0.1", 4);
            }
          };
        },
      ],
    },
    new crawlee.Configuration({ persistStorage: false, purgeOnStart: true }),
  );
  const started = performance.now();
  await crawler.run(urls);
  const elapsed = (performance.now() - started) / 1000;
  return { kind: "crawlee", pages, seconds: elapsed, pages_per_second: pages / elapsed };
}

const require = createRequire(pathToFileURL(`${crawleeFrom}/`));
const crawleeModule: unknown = await import(pathToFileURL(require.resolve("crawlee")).href);
const crawlee = isCrawlee(crawleeModule) ? crawleeModule : fail("crawlee has no HttpCrawler");

const loadAtStart = os.loadavg()[0];
const trials: Trial[] = [];
let version = "";
for (let trial = 0; trial < trialCount; trial += 1) {
  for (const kind of ["proving_ground", "crawlee"] as const) {
    const serving = await serve();
    version = serving.version;
    const measured =
      kind === "proving_ground"
        ? await provingGroundTrial(serving)
        : await crawleeTrial(serving, crawlee);
    trials.push(measured);
    await serving.stop();
    console.error(JSON.stringify(measured));
  }
}
const rates = (kind: Trial["kind"]) =>
  trials.filter((trial) => trial.kind === kind).map((trial) => trial.pages_per_second);
const slowestServed = Math.min(...rates("proving_ground"));
const fastestConsumed = Math.max(...rates("crawlee"));
const cpus = os.cpus();
const ran = new Date().toISOString();
const record = {
  declaration: "declaration.json",
  ran,
  proving_ground_version: version,
  commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  host: {
    class: "local-controlled",
    platform: `${os.platform()} ${os.release()}`,
    arch: os.arch(),
    cpu: cpus[0]?.model ?? "unknown",
    cpus: cpus.length,
    memory_bytes: os.totalmem(),
    load_average_1m_at_start: loadAtStart,
    node: process.version,
  },
  trials,
  slowest_served_pages_per_second: slowestServed,
  fastest_consumed_pages_per_second: fastestConsumed,
  outcome: slowestServed > fastestConsumed ? "met" : "not_met",
};
mkdirSync(new URL("runs/", here), { recursive: true });
const name = `runs/${ran.slice(0, 10)}-local-controlled.json`;
writeFileSync(new URL(name, here), `${JSON.stringify(record, null, 2)}\n`);
console.log(JSON.stringify({ outcome: record.outcome, slowestServed, fastestConsumed, name }));
