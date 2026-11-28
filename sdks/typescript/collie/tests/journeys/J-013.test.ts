// J-013, "Run a one-off SDK script on an Attached engine, then two scripts
// that share the Daemon", on the `sdk-typescript` surface (ADRs 0063, 0075
// and 0094): a one-off script with no Daemon running starts an Attached
// engine over stdio that stops when it ends without starting a Daemon; then
// once the Daemon is started on the CLI, two scripts run at once and share
// that one Daemon without starting an Attached engine. Rust's
// `tests/it/journeys/j013.rs` and Python's `tests/journeys/test_j013.py`
// are its twins.
//
// Covers: row:RT-19, story:3-1, story:3-2
import { Collie, type EngineInfo, type PageField } from "@goodfetch/collie";
import { journey, taskStartUrl, test, type JourneyRun } from "@goodfetch/test-support";
import { expect } from "vitest";

/** The product's ID, name and price, as task `catalog/2` names them. */
const PRODUCT_FIELDS: Readonly<Record<string, PageField>> = {
  id: { locator: { kind: "css", selector: "span.id" } },
  name: { locator: { kind: "css", selector: "h1.name" } },
  price: {
    locator: { kind: "css", selector: "span.price" },
    attribute: "data-price",
    transform: { kind: "number", locale: "en" },
  },
};

interface ScriptOutcome {
  readonly engine: EngineInfo;
  readonly record: Readonly<Record<string, unknown>>;
}

/** Collects a stream's UTF-8 text until it ends. */
async function readStream(stream: NodeJS.ReadableStream): Promise<string> {
  let text = "";
  stream.setEncoding("utf8");
  for await (const chunk of stream) {
    text += String(chunk);
  }
  return text;
}

/** Runs Collie's CLI with `args` and `--json`, requiring exit 0 and JSON output. */
async function cliJson(run: JourneyRun, args: readonly string[]): Promise<object> {
  const proc = run.command([...args, "--json"]);
  proc.child.stdin.end();
  const [stdout, stderr, exit] = await Promise.all([
    readStream(proc.child.stdout),
    readStream(proc.child.stderr),
    proc.exit,
  ]);
  if (exit.code !== 0) {
    throw new Error(
      `collie ${args.join(" ")} exited with ${String(exit.code)} (${String(exit.signal)}): ${stderr}`,
    );
  }
  const parsed: unknown = JSON.parse(stdout);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new TypeError(`expected JSON object from collie ${args.join(" ")}, got: ${stdout}`);
  }
  return parsed;
}

/** Runs one SDK script that extracts the catalog's product page and closes cleanly. */
async function extractProduct(
  env: Readonly<Record<string, string>>,
  productUrl: string,
): Promise<ScriptOutcome> {
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  const collie = await Collie.connect({ env });
  try {
    const engine = collie.engine();
    const started = await collie.start();
    const page = await started.newPage();
    await page.goto(productUrl);
    const read = await page.read(PRODUCT_FIELDS);
    await started.finish();
    return { engine, record: read.values };
  } finally {
    await collie.close();
  }
}

test(
  "J-013@sdk-typescript",
  { tags: ["J-013", "sdk-typescript"] },
  journey(
    async (run) => {
      const provingGround = run.provingGround;
      if (provingGround === undefined) {
        throw new TypeError("J-013 has a Proving ground");
      }
      const product = await taskStartUrl(provingGround, "catalog/2");
      let daemonStarted = false;
      let statusAfterOneOff: object | undefined;
      let daemonStartPid: unknown;
      let statusAfterShared: object | undefined;

      try {
        const oneOff = await run.step(
          "With no Daemon running and nothing configured, run a one-off script that extracts the catalog's product page",
          async () => {
            const outcome = await extractProduct(run.env, product);
            statusAfterOneOff = await cliJson(run, ["daemon", "status"]);
            return outcome;
          },
        );

        await run.step("Start the Daemon", async () => {
          const started = await cliJson(run, ["daemon", "start"]);
          daemonStarted = true;
          daemonStartPid = Reflect.get(started, "pid");
          expect(typeof daemonStartPid).toBe("number");
        });

        const shared = await run.step(
          "Run two scripts at once, each extracting the catalog's product page",
          async () => {
            const outcomes = await Promise.all([
              extractProduct(run.env, product),
              extractProduct(run.env, product),
            ]);
            statusAfterShared = await cliJson(run, ["daemon", "status"]);
            return outcomes;
          },
        );

        await run.check(
          "The one-off script ran on an Attached engine it started over stdio, which stopped when the script ended, and no Daemon was started",
          () => {
            expect(oneOff?.engine.kind).toBe("attached");
            expect(oneOff?.engine.enginePath).not.toBeNull();
            expect(
              statusAfterOneOff === undefined ? undefined : Reflect.get(statusAfterOneOff, "pid"),
            ).toBeNull();
          },
        );

        await run.expectTask("The oracle verifies the one-off script's result", {
          records: oneOff === undefined ? [] : [oneOff.record],
        });

        await run.check(
          "Both scripts ran on the one running Daemon, and neither started an Attached engine",
          () => {
            expect(shared).toBeDefined();
            if (shared === undefined) {
              return;
            }
            const [first, second] = shared;
            expect(first.engine.kind).toBe("daemon");
            expect(second.engine.kind).toBe("daemon");
            expect(first.engine.enginePath).toBeNull();
            expect(second.engine.enginePath).toBeNull();
            expect(first.engine.url).not.toBeNull();
            expect(second.engine.url).toBe(first.engine.url);
            expect(
              statusAfterShared === undefined ? undefined : Reflect.get(statusAfterShared, "pid"),
            ).toBe(daemonStartPid);
          },
        );

        await run.expectTask("The oracle verifies the first script's result", {
          records: shared === undefined ? [] : [shared[0].record],
        });

        await run.expectTask("The oracle verifies the second script's result", {
          records: shared === undefined ? [] : [shared[1].record],
        });
      } finally {
        if (daemonStarted) {
          const stop = run.command(["daemon", "stop", "--json"]);
          stop.child.stdin.end();
          await Promise.all([
            readStream(stop.child.stdout),
            readStream(stop.child.stderr),
            stop.exit,
          ]);
        }
      }
    },
    { browser: true },
  ),
);
