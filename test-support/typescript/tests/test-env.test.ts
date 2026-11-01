// Seam B (spec #325): the `testEnv` fixture, used exactly as a product test
// uses it, against the real `goodfetch-test-env` the tool manifest names;
// and `startTestEnv`, on which it is built, against a stand-in that
// misbehaves (`fixtures/fake-test-env.mjs`).
import { existsSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect } from "vitest";
import { ProcessRecord, redact, startTestEnv, test, type Program } from "@goodfetch/test-support";

const FAKE_TEST_ENV = fileURLToPath(new URL("fixtures/fake-test-env.mjs", import.meta.url));

/** The stand-in `goodfetch-test-env`, misbehaving as `behaviour` names. */
function fake(behaviour: string): Program {
  return { command: process.execPath, args: [FAKE_TEST_ENV, behaviour] };
}

describe("testEnv", () => {
  test("describes the test's folder, home and cleared environment", ({ testEnv }) => {
    expect(existsSync(testEnv.folder)).toBe(true);
    expect(testEnv.home).toBe(join(testEnv.folder, "home"));
    expect(testEnv.env["GOODFETCH_HOME"]).toBe(testEnv.home);
    expect(testEnv.env["TZ"]).toBe("UTC");
    expect(testEnv.start_time).toBe("2026-01-05T09:00:00Z");
  });

  test("its folder redacts in a snapshot as written, canonical and in JSON", ({ testEnv }) => {
    const output = [
      `${testEnv.folder}/a`,
      `${realpathSync(testEnv.folder)}/b`,
      JSON.stringify({ path: join(testEnv.folder, "c") }),
    ].join(" ");

    expect(redact(output, { folder: testEnv.folder })).toBe(
      `[FOLDER]/a [FOLDER]/b ${JSON.stringify({ path: join("[FOLDER]", "c") })}`,
    );
  });

  test("teardown of a failed test keeps its folder", async ({ processes, toolManifest }) => {
    const started = await startTestEnv({ processes, testName: "a failed test", toolManifest });
    const { folder } = started.description;

    await started.close({ failed: true });

    expect(existsSync(folder)).toBe(true);
    rmSync(folder, { recursive: true, force: true });
  });

  test("teardown removes the folder once goodfetch-test-env has ended", ({
    testEnv,
    onTestFinished,
  }) => {
    onTestFinished(() => {
      expect(existsSync(testEnv.folder)).toBe(false);
    });

    expect(existsSync(testEnv.folder)).toBe(true);
  });
});

/** One meeting of the two tests below: what each run brought to it. */
interface Meeting {
  readonly folders: string[];
  readonly homes: string[];
  readonly both: Promise<void>;
  readonly release: () => void;
}

function meeting(): Meeting {
  const { promise: both, resolve: release } = Promise.withResolvers<void>();
  return { folders: [], homes: [], both, release };
}

// Two tests running at once each get a folder and home of their own: each
// waits until both have their setup, so a run that does not hold both at
// once times out rather than pass. Each pair of runs meets anew, so a
// repeated run (`just repeat`) compares only its own pair: neither test's
// next run starts before the other has joined their meeting.
describe.concurrent("testEnv of two tests running at once", () => {
  let current = meeting();

  for (const name of ["first", "second"]) {
    test(`gives the ${name} test a folder and home of its own`, async ({ testEnv }) => {
      const joined = current;
      joined.folders.push(testEnv.folder);
      joined.homes.push(testEnv.home);
      if (joined.folders.length === 2) {
        current = meeting();
        joined.release();
      }
      await joined.both;

      expect(new Set(joined.folders).size).toBe(2);
      expect(new Set(joined.homes).size).toBe(2);
    });
  }
});

describe("startTestEnv", () => {
  test("gives two setups started at once different folders and homes", async ({
    processes,
    toolManifest,
  }) => {
    const start = { processes, testName: "two setups at once", toolManifest };

    const [first, second] = await Promise.all([startTestEnv(start), startTestEnv(start)]);

    expect(first.description.folder).not.toBe(second.description.folder);
    expect(first.description.home).not.toBe(second.description.home);
    await Promise.all([first.close(), second.close()]);
  });

  test("refuses a description that fails the committed schema", async () => {
    const processes = new ProcessRecord();

    const started = startTestEnv({
      processes,
      testName: "refuses a bad description",
      toolManifest: undefined,
      program: fake("bad-description"),
    });

    await expect(started).rejects.toMatchObject({ code: "description_invalid" });
    await processes.close();
  });

  test("fails a start that ends before describing the setup", async () => {
    const processes = new ProcessRecord();

    const started = startTestEnv({
      processes,
      testName: "a failed start",
      toolManifest: undefined,
      program: fake("fails-to-start"),
    });

    await expect(started).rejects.toMatchObject({ code: "start_failed" });
    await processes.close();
  });

  test("fails a start whose program cannot run, with the spawn's error as its cause", async () => {
    const processes = new ProcessRecord();

    const started = startTestEnv({
      processes,
      testName: "a missing program",
      toolManifest: undefined,
      program: { command: "/nonexistent/goodfetch-test-env", args: [] },
    });

    await expect(started).rejects.toMatchObject({
      code: "start_failed",
      cause: expect.objectContaining({ code: "ENOENT" }),
    });
    await processes.close();
  });

  test("fails a teardown that goodfetch-test-env ends with a failure", async () => {
    const processes = new ProcessRecord();
    const started = await startTestEnv({
      processes,
      testName: "a failed teardown",
      toolManifest: undefined,
      program: fake("fails-teardown"),
    });

    const closed = started.close();

    await expect(closed).rejects.toMatchObject({ code: "teardown_failed" });
    await processes.close();
  });

  test("leaves a goodfetch-test-env alive past the teardown deadline to the process record", async () => {
    const processes = new ProcessRecord();
    const started = await startTestEnv({
      processes,
      testName: "a teardown past its deadline",
      toolManifest: undefined,
      program: fake("stays-alive"),
      teardownDeadlineMs: 100,
    });
    await started.close();

    const closed = processes.close();

    await expect(closed).rejects.toMatchObject({ code: "processes_alive" });
  });

  test("fails without a test tool manifest", async () => {
    const started = startTestEnv({
      processes: new ProcessRecord(),
      testName: "no manifest",
      toolManifest: undefined,
    });

    await expect(started).rejects.toMatchObject({ code: "tools_unset" });
  });

  test.for([
    { name: "lists no goodfetch-test-env", manifest: {}, code: "tool_missing" },
    { name: "fails its schema", manifest: { test_env: 3 }, code: "tools_invalid" },
  ])("fails with a test tool manifest that $name", async ({ manifest, code }, { testEnv }) => {
    const path = join(testEnv.folder, "test-tools.json");
    writeFileSync(path, JSON.stringify(manifest));

    const started = startTestEnv({
      processes: new ProcessRecord(),
      testName: "bad manifest",
      toolManifest: path,
    });

    await expect(started).rejects.toMatchObject({ code });
  });
});
