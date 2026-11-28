// J-032, "Crawl a site whose certificate is invalid by listing its domain
// for trust on first use, and see the crawl stop when the key changes until
// I accept it", on the `sdk-typescript` surface (ADRs 0042, 0063 and 0075;
// spec #565, ticket #688): crawls `expired.certificates.test` through
// `@goodfetch/foxhound` with `tls.accept_invalid_certs`, verifies the crawl
// stops on a changed key with `certificate_changed`, verifies `certs.accept`
// is refused both through the SDK (`person_required`) and through non-interactive
// `--json`, accepts the new key in a terminal (`certs accept`), and crawls again.
//
// Covers: story:4-35, story:4-36, story:4-104, row:SC-06
import { spawn } from "node:child_process";
import {
  Crawl,
  Foxhound,
  GoodfetchError,
  seed,
  type CrawlRequestOutcome,
  type CrawlSettings,
  type DatasetRecord,
} from "@goodfetch/foxhound";
import {
  journey,
  ProcessRecord,
  provingGroundObservations,
  startTestEnv,
  test,
  type ProvingGroundDescription,
  type RecordedProcess,
  type StartedTestEnv,
} from "@goodfetch/test-support";
import { expect } from "vitest";

const HOST = "expired.certificates.test";
const CHANGED_SEED = 1032;

interface PageRecord {
  readonly url: string;
}

interface Crawled {
  readonly outcome: {
    readonly kind: string;
    readonly errors: readonly string[];
  };
  readonly pinnedKey: string | null;
  readonly recordCount: number;
}

function siteUrl(provingGround: ProvingGroundDescription): string {
  const port = provingGround.address.slice(provingGround.address.lastIndexOf(":") + 1);
  return `https://${HOST}:${port}/`;
}

function settingsFor(provingGround: ProvingGroundDescription): CrawlSettings {
  const resolve: Record<string, readonly string[]> = {};
  for (const [host, address] of Object.entries(provingGround.resolve)) {
    resolve[host] = [address];
  }
  return {
    fetch: { tier: "http" },
    politeness: { domain: { delay: 0 } },
    sitePreferences: { respectRobotsTxt: false },
    network: { resolve },
    egress: { privateRanges: Object.keys(resolve) },
    tls: { acceptInvalidCerts: { "certificates.test": null } },
  };
}

async function runCrawl(
  foxhound: Foxhound,
  provingGround: ProvingGroundDescription,
  runId: string,
): Promise<Crawled> {
  const crawl = new Crawl();
  const page = crawl.label("page");
  const pages = crawl.dataset<PageRecord>("pages");
  crawl.handle(page, async (context) => {
    await context.push(pages, { url: context.request.url });
  });

  const active = await foxhound.startCrawl(crawl, {
    runId,
    seeds: [seed(siteUrl(provingGround), page)],
    settings: settingsFor(provingGround),
  });
  const report = await active.finish();
  expect(report.lifecycle).toBe("finished");

  const outcomes: CrawlRequestOutcome[] = [];
  for await (const outcome of active.outcomes()) {
    outcomes.push(outcome);
  }
  const firstOutcome = outcomes[0];
  if (firstOutcome === undefined) {
    throw new TypeError(`crawl ${runId} returned no outcomes`);
  }

  const records: DatasetRecord<PageRecord>[] = [];
  for await (const record of active.records(pages)) {
    records.push(record);
  }
  const mark = records[0]?.provenance.certificateException;
  const pinnedKey =
    records.length === 1 &&
    mark !== null &&
    mark !== undefined &&
    mark.kind === "trust_on_first_use"
      ? mark.spkiSha256
      : null;

  return {
    outcome: {
      kind: firstOutcome.kind,
      errors: firstOutcome.errors.map((error) => error.code),
    },
    pinnedKey,
    recordCount: records.length,
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

/** Runs `certs accept <host>` in a real PTY and answers `y` to `[y/N]`. */
async function acceptCertInTerminal(
  executable: string,
  host: string,
  env: Readonly<Record<string, string>>,
  cwd: string,
): Promise<{ readonly code: number | null; readonly output: string }> {
  const script = [
    "import os, pty, select, subprocess, sys, time",
    "master, slave = pty.openpty()",
    "proc = subprocess.Popen(sys.argv[1:], stdin=slave, stdout=slave, stderr=slave, close_fds=True)",
    "os.close(slave)",
    "buf = bytearray()",
    "sent = False",
    "deadline = time.monotonic() + 30.0",
    "while time.monotonic() < deadline:",
    "    r, _, _ = select.select([master], [], [], 0.1)",
    "    if r:",
    "        try:",
    "            chunk = os.read(master, 4096)",
    "        except OSError:",
    "            break",
    "        if not chunk:",
    "            break",
    "        buf.extend(chunk)",
    "        if not sent and b'[y/N]' in buf:",
    "            os.write(master, b'y\\n')",
    "            sent = True",
    "    elif proc.poll() is not None:",
    "        break",
    "os.close(master)",
    "sys.stdout.buffer.write(buf)",
    "sys.exit(proc.wait(timeout=10))",
  ].join("\n");

  const child = spawn("python3", ["-c", script, executable, "certs", "accept", host], {
    env,
    cwd,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let output = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    output += chunk;
  });
  child.stderr.on("data", (chunk: string) => {
    output += chunk;
  });
  const code = await new Promise<number | null>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (exitCode) => resolve(exitCode));
  });
  return { code, output };
}

test(
  "J-032@sdk-typescript",
  {
    tags: ["J-032", "sdk-typescript", "story:4-35", "story:4-36", "story:4-104", "row:SC-06"],
  },
  journey(async (run) => {
    const provingGround = run.provingGround;
    if (provingGround === undefined) {
      throw new TypeError("J-032 has a Proving ground");
    }

    const extraProcesses = new ProcessRecord();
    let changedEnv: StartedTestEnv | undefined;
    let foxhound: Foxhound | undefined;
    let first: Crawled | undefined;
    let changed: Crawled | undefined;
    let changedRequests = Number.MAX_SAFE_INTEGER;
    let sdkAcceptErrorCode = "";
    let jsonAcceptExit: number | null = 0;
    let accepted: Crawled | undefined;

    try {
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078).
      foxhound = await Foxhound.connect({ env: run.env });
      const client = foxhound;

      await run.step(
        "Crawl the site from a program, listing its domain for trust on first use in the crawl's settings",
        async () => {
          first = await runCrawl(client, provingGround, "first-use");
        },
      );

      await run.step(
        "Crawl the site the same way once its server presents another key",
        async () => {
          changedEnv = await startTestEnv({
            processes: extraProcesses,
            testName: "J-032-sdk-typescript-changed-key",
            toolManifest: process.env["GOODFETCH_TEST_TOOLS"],
            provingGroundSeed: CHANGED_SEED,
          });
          const changedServer = changedEnv.description.proving_ground;
          if (changedServer === undefined) {
            throw new TypeError("changed-key Proving ground started");
          }
          changed = await runCrawl(client, changedServer, "changed-key");
          const observations = await provingGroundObservations(changedServer);
          changedRequests = observations.requests.filter((request) => request.host === HOST).length;

          // Verify `certs.accept` is refused through the SDK (`person_required`).
          const listed: unknown = await client.call("certs.list", {});
          const pins =
            typeof listed === "object" && listed !== null ? Reflect.get(listed, "pins") : undefined;
          let presented = "";
          if (Array.isArray(pins)) {
            for (const pin of pins) {
              if (typeof pin === "object" && pin !== null && Reflect.get(pin, "host") === HOST) {
                const value: unknown = Reflect.get(pin, "changed_spki_sha256");
                if (typeof value === "string") {
                  presented = value;
                }
              }
            }
          }
          expect(presented.length).toBeGreaterThan(0);

          try {
            await client.call("certs.accept", { host: HOST, spki_sha256: presented });
            sdkAcceptErrorCode = "answered";
          } catch (error: unknown) {
            sdkAcceptErrorCode = error instanceof GoodfetchError ? error.code : String(error);
          }

          // Verify non-interactive `--json` CLI path also refuses `certs accept`.
          const jsonResult = await readOutput(run.command(["certs", "accept", HOST, "--json"]));
          jsonAcceptExit = jsonResult.code;
        },
      );

      await run.step("Accept the site's new key in a terminal (`certs accept`)", async () => {
        const answered = await acceptCertInTerminal(
          run.engine,
          HOST,
          run.testEnv.env,
          run.testEnv.folder,
        );
        expect(answered.code).toBe(0);
      });

      await run.step("Crawl the site the same way again", async () => {
        const changedServer = changedEnv?.description.proving_ground;
        if (changedServer === undefined) {
          throw new TypeError("changed-key Proving ground running");
        }
        accepted = await runCrawl(client, changedServer, "accepted");
      });
    } finally {
      if (foxhound !== undefined) {
        await foxhound.close();
      }
      if (changedEnv !== undefined) {
        await changedEnv.close();
      }
      await extraProcesses.close();
    }

    await run.check(
      "The first crawl fetched the site through its exception, and its Record's Provenance carries the exception and the key it pinned",
      () => {
        expect(first?.outcome).toEqual({ kind: "succeeded", errors: [] });
        expect(first?.pinnedKey).toBeTruthy();
      },
    );

    await run.check(
      "The crawl after the key changed failed the site's request with `certificate_changed`, and no request reached the site",
      () => {
        expect(changed?.outcome).toEqual({ kind: "failed", errors: ["certificate_changed"] });
        expect(changed?.recordCount).toBe(0);
        expect(changedRequests).toBe(0);
        expect(sdkAcceptErrorCode).toBe("person_required");
        expect(jsonAcceptExit).not.toBe(0);
      },
    );

    await run.check(
      "Once the new key was accepted, the crawl fetched the site under the new key",
      () => {
        expect(accepted?.outcome).toEqual({ kind: "succeeded", errors: [] });
        expect(accepted?.pinnedKey).toBeTruthy();
        expect(accepted?.pinnedKey).not.toBe(first?.pinnedKey);
      },
    );
  }),
);
