// Collie's testing kit (issue 3398, ADR 0105), on the real engine:
// `TestEngine.start()` starts an Attached engine of Collie's executable of
// the same commit in a temporary Goodfetch home it removes on close, with a
// cleared environment and `egress.offline` on; a host `network.resolve`
// maps to loopback alone is reached live and any other is refused, and
// browsers come from the caller's Browser cache, which the engine reads and
// never writes: the kit turns `browser_cache.read_only` on (issue 3458).
// With a Daemon running in the caller's home, the code under test runs
// unchanged on the Daemon's connection and on the test engine's, which
// reaches its own Attached engine. The kit's sample test, under
// `fixtures/kit-sample/`, runs under Vitest and `node:test` with no plugin of
// Goodfetch's (issue 3399). Rust's `tests/it/sdk/test_engine.rs` and
// Python's `test_test_engine.py` are its twins.
import { readFile, lstat, readdir, stat } from "node:fs/promises";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { ArtifactUnavailableOfflineError, Collie, EgressRefusedError } from "@goodfetch/collie";
import { TestEngine } from "@goodfetch/collie/testing";
import {
  failure,
  provingGroundUrl,
  taskStartUrl,
  test,
  type ProcessRecord,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The folder of the kit's sample test and each runner's file for it. */
const KIT_SAMPLE = join(import.meta.dirname, "fixtures", "kit-sample");

/** `name`'s command-line program at `bin` in its package, the workspace's own. */
function runner(name: string, bin: string): string {
  return join(dirname(fileURLToPath(import.meta.resolve(`${name}/package.json`))), bin);
}

/** How a runner's process over the kit's sample test ended, and what it wrote. */
interface SampleRun {
  readonly exit: { readonly code: number | null; readonly signal: string | null };
  readonly stdout: string;
  readonly stderr: string;
}

/**
 * `args` run by Node in the kit's sample folder with the test's
 * environment, from which the sample's engine comes, as a test run sets it.
 */
async function runSample(
  processes: ProcessRecord,
  env: Readonly<Record<string, string>>,
  args: readonly string[],
): Promise<SampleRun> {
  const program = processes.spawn(process.execPath, args, { env, cwd: KIT_SAMPLE });
  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  program.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
  program.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
  program.child.stdin.end();
  const { code, signal } = await program.exit;
  return {
    exit: { code, signal },
    stdout: Buffer.concat(stdout).toString(),
    stderr: Buffer.concat(stderr).toString(),
  };
}

/**
 * A host the test's `network.resolve` maps to an address that is not
 * loopback, TEST-NET-1's, which no Run reaches.
 */
const PUBLIC_HOST = "public.test";

/** An entry of a Browser cache: whether it is a folder, its length and modification time. */
type Entry = readonly [boolean, number, number];

/**
 * Every entry of the Browser cache at `cache`, by its path under `cache`:
 * its kind, length and modification time, symbolic links unfollowed. The
 * cache's own records, and not a build's `files` folder, where no engine
 * writes and the browser it launches keeps its own folders.
 */
async function tree(cache: string): Promise<Record<string, Entry>> {
  const entries: Record<string, Entry> = {};
  const walk = async (folder: string): Promise<void> => {
    const depth = relative(cache, folder)
      .split(sep)
      .filter((part) => part !== "").length;
    for (const child of await readdir(folder, { withFileTypes: true })) {
      if (child.isDirectory() && child.name === "files" && depth === 3) {
        continue;
      }
      const path = join(folder, child.name);
      const status = await lstat(path);
      entries[relative(cache, path).split(sep).join("/")] = [
        status.isDirectory(),
        status.size,
        status.mtimeMs,
      ];
      if (status.isDirectory()) {
        await walk(path);
      }
    }
  };
  await walk(cache);
  return entries;
}

/** Whether `path` names something on disk. */
async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

describe("a test engine", () => {
  test(
    "runs a test's code on its connection, and close removes its home",
    {
      tags: [
        "client_case/test-engine-runs-the-code-under-test",
        "attached",
        "story:11-156",
        "story:11-157",
        "story:11-168",
      ],
    },
    async ({ env, clientCase }) => {
      const engine = await TestEngine.start({ env });
      const connection = engine.connection;
      const home = connection.engine().home;
      if (home === null) {
        throw new TypeError("an Attached engine names its home");
      }
      const existed = await exists(home);

      const report = await (await connection.start()).finish();
      await engine.close();

      await clientCase.check("engine.connection is the Product's own connection type", () =>
        expect(connection).toBeInstanceOf(Collie),
      );
      await clientCase.check("the code under test finishes a Run on engine.connection", () =>
        expect(report.lifecycle).toBe("finished"),
      );
      await clientCase.check(
        "the engine ran in a temporary Goodfetch home of its own, removed on close",
        async () => {
          expect(home).not.toBe(env["GOODFETCH_HOME"]);
          expect([existed, await exists(home)]).toEqual([true, false]);
        },
      );
    },
  );

  describe("beside a Daemon in the caller's home", () => {
    test.override("withDaemon", true);

    test(
      "runs the code under test unchanged on the Daemon's connection and on its own",
      {
        tags: [
          "client_case/test-engine-runs-the-code-under-test",
          "daemon",
          "story:11-157",
          "story:11-168",
        ],
      },
      async ({ env, testEnv, daemon, clientCase }) => {
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        const collie = await Collie.connect({ env, engine: "daemon" });
        const onTheDaemon = (await (await collie.start()).finish()).lifecycle;
        const reachedDaemon = collie.engine();
        await collie.close();
        const engine = await TestEngine.start({ env });
        const onTheTestEngine = (await (await engine.connection.start()).finish()).lifecycle;
        const reached = engine.connection.engine();
        await engine.close();

        await clientCase.check(
          "the code under test finishes a Run on a Daemon's connection and on the test engine's",
          () => expect([onTheDaemon, onTheTestEngine]).toEqual(["finished", "finished"]),
        );
        await clientCase.check(
          "the test engine reaches its own Attached engine in its own home, never the Daemon of the caller's",
          () => {
            expect([reachedDaemon.kind, reachedDaemon.url]).toEqual(["daemon", daemon.socket]);
            expect(reached).toMatchObject({ kind: "attached", url: null });
            expect(reached.home).not.toBe(testEnv.home);
          },
        );
      },
    );
  });

  describe("the kit's sample test", () => {
    test(
      "passes under Vitest with no plugin of Goodfetch's",
      { tags: ["attached", "story:11-156", "story:11-171"] },
      async ({ env, testEnv, processes }) => {
        const report = join(testEnv.folder, "vitest.json");

        const ran = await runSample(processes, env, [
          runner("vitest", "vitest.mjs"),
          "run",
          "--config",
          join(KIT_SAMPLE, "vitest.config.ts"),
          "--reporter=json",
          `--outputFile=${report}`,
        ]);

        // The received output shows in the diff when the run failed.
        expect({ ...ran, report: JSON.parse(await readFile(report, "utf8")) }).toMatchObject({
          exit: { code: 0, signal: null },
          report: { numTotalTests: 1, numPassedTests: 1 },
        });
      },
    );

    test(
      "passes under node:test",
      { tags: ["attached", "story:11-156", "story:11-171"] },
      async ({ env, processes }) => {
        const ran = await runSample(processes, env, [
          "--test",
          "--test-reporter=tap",
          "sample.node-test.ts",
        ]);

        expect({
          ...ran,
          counts: [...ran.stdout.matchAll(/^# (tests|pass|fail) (\d+)$/gmu)].map(
            ([, name, count]) => [name, count],
          ),
        }).toMatchObject({
          exit: { code: 0, signal: null },
          counts: [
            ["tests", "1"],
            ["pass", "1"],
            ["fail", "0"],
          ],
        });
      },
    );
  });

  describe("on the Proving ground", () => {
    test.override("provingGroundSeed", 3398);

    describe("with no browser installed", () => {
      // A mirror it never reaches: `egress.offline` refuses the download first.
      test.override("browserDownload", "not-found");

      test(
        "fails the first page with artifact_unavailable_offline, the cache left empty",
        { tags: ["attached", "story:11-169"] },
        async ({ env }) => {
          const cache = env["GOODFETCH_BROWSERS_PATH"];
          if (cache === undefined) {
            throw new TypeError("the test's environment names its Browser cache");
          }
          const engine = await TestEngine.start({ env });
          const run = await engine.connection.start();
          const refused = await failure(run.newPage());
          await run.finish();
          await engine.close();

          expect(refused).toBeInstanceOf(ArtifactUnavailableOfflineError);
          expect(refused).toHaveProperty(
            "fix",
            "Run `goodfetch-collie browser install` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set",
          );
          expect(await tree(cache)).toEqual({});
        },
      );
    });

    describe("with a browser installed", () => {
      test.override("withBrowser", true);

      // The caller's Browser cache stays writable here: the kit's
      // `browser_cache.read_only` alone keeps the engine from recording the
      // build's use in it (ADR 0098).
      test(
        "reaches a loopback resolve entry live, refuses any other host, and leaves the cache unchanged",
        { tags: ["attached", "story:11-168", "story:11-169"] },
        async ({ env, provingGround }) => {
          const callerHome = env["GOODFETCH_HOME"];
          if (callerHome === undefined) {
            throw new TypeError("the test's environment names its Goodfetch home");
          }
          const cache = join(callerHome, "browsers");
          const resolve = Object.fromEntries(
            Object.entries(provingGround.resolve).map(([host, address]) => [host, [address]]),
          );
          const reached = await taskStartUrl(provingGround, "statuses/1");
          const before = await tree(cache);
          const engine = await TestEngine.start({
            env,
            settings: { network: { resolve: { ...resolve, [PUBLIC_HOST]: ["192.0.2.7"] } } },
          });
          const run = await engine.connection.start();
          const page = await run.newPage();

          const status = (await page.goto(reached))?.status();
          const refused = await failure(
            page.goto(provingGroundUrl(provingGround, PUBLIC_HOST, "/")),
          );
          await run.finish();
          await engine.close();

          expect([status, refused]).toEqual([200, expect.any(EgressRefusedError)]);
          expect(await tree(cache)).toEqual(before);
        },
      );
    });
  });
});
