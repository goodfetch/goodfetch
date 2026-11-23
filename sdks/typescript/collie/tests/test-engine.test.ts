// Collie's testing kit (issue 3398, ADR 0105), on the real engine:
// `TestEngine.start()` starts an Attached engine of Collie's executable of
// the same commit in a temporary Goodfetch home it removes on close, with a
// cleared environment and `egress.offline` on; a host `network.resolve`
// maps to loopback alone is reached live and any other is refused, and
// browsers come from the caller's Browser cache, which the engine reads and
// never writes: the kit turns `browser_cache.read_only` on (issue 3458).
// Rust's `tests/it/sdk/test_engine.rs` and Python's `test_test_engine.py`
// are its twins.
import { lstat, readdir, stat } from "node:fs/promises";
import { join, relative, sep } from "node:path";
import { ArtifactUnavailableOfflineError, EgressRefusedError } from "@goodfetch/collie";
import { TestEngine } from "@goodfetch/collie/testing";
import { failure, provingGroundUrl, taskStartUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

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
    { tags: ["attached"] },
    async ({ env }) => {
      const engine = await TestEngine.start({ env });
      const home = engine.connection.engine().home;
      if (home === null) {
        throw new TypeError("an Attached engine names its home");
      }
      const existed = await exists(home);

      const run = await engine.connection.start();
      const report = await run.finish();
      await engine.close();

      expect(report.lifecycle).toBe("finished");
      expect(home).not.toBe(env["GOODFETCH_HOME"]);
      expect([existed, await exists(home)]).toEqual([true, false]);
    },
  );

  describe("on the Proving ground", () => {
    test.override("provingGroundSeed", 3398);

    describe("with no browser installed", () => {
      // A mirror it never reaches: `egress.offline` refuses the download first.
      test.override("browserDownload", "not-found");

      test(
        "fails the first page with artifact_unavailable_offline, the cache left empty",
        { tags: ["attached"] },
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
        { tags: ["attached"] },
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
