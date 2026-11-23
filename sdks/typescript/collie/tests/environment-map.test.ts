// `connect()`'s environment map (ADR 0083) on the Scripted engine as the
// Attached engine, which writes the environment it was started with: the
// engine gets exactly the `env` map, and with none the process
// environment, shown in a program the test starts with an environment of
// its own, so no test patches its own (ADR 0062). A `GOODFETCH_` variable
// off the list, an empty one and one the engine cannot parse fail
// `connect()` before any engine starts. Python's `test_environment_map.py`
// is its twin.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Collie } from "@goodfetch/collie";
import {
  failure,
  scriptedEngineEnv,
  scriptedEngineEnvironment,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const SCRIPT = "connect/reached-attached.json";

/** A variable outside the reserved prefix, which the map carries as it is. */
const MARK = "ENVIRONMENT_MAP_MARK";

/**
 * The variables libuv adds on Windows to a child's environment that lacks
 * them, from its parent's (`required_vars` of its `src/win/process.c`), so
 * Windows can start the process.
 */
const ADDED_ON_WINDOWS = [
  "HOMEDRIVE",
  "HOMEPATH",
  "LOGONSERVER",
  "PATH",
  "SYSTEMDRIVE",
  "SYSTEMROOT",
  "TEMP",
  "USERDOMAIN",
  "USERNAME",
  "USERPROFILE",
  "WINDIR",
];

/**
 * The environment `started` without the variables Node's spawn added to
 * `given`, a map: none outside Windows.
 */
function asGiven(
  started: Readonly<Record<string, string>> | undefined,
  given: Readonly<Record<string, string>>,
): Readonly<Record<string, string>> | undefined {
  if (started === undefined || process.platform !== "win32") {
    return started;
  }
  const named = new Set(Object.keys(given).map((name) => name.toUpperCase()));
  return Object.fromEntries(
    Object.entries(started).filter(
      ([name]) => named.has(name.toUpperCase()) || !ADDED_ON_WINDOWS.includes(name.toUpperCase()),
    ),
  );
}

describe("connect()'s environment map", () => {
  test(
    "the Attached engine gets exactly the env map, or the process environment, and a bad GOODFETCH_ variable reaches none",
    {
      tags: [
        "client_case/connect-takes-the-environment-map",
        "attached",
        "story:11-73",
        "error:setting_unknown",
        "error:setting_invalid",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const givenHome = join(testEnv.folder, "given");
      const given = { ...scriptedEngineEnv(testEnv, "collie", SCRIPT, givenHome), [MARK]: "given" };
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: given });
      await collie.close();

      const processHome = join(testEnv.folder, "process");
      const out = join(testEnv.folder, "environment.json");
      const program = processes.spawn(
        process.execPath,
        [join(import.meta.dirname, "fixtures", "environment.ts"), out],
        {
          env: { ...scriptedEngineEnv(testEnv, "collie", SCRIPT, processHome), [MARK]: "process" },
          cwd: testEnv.folder,
        },
      );
      const stderr: Buffer[] = [];
      program.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
      program.child.stdin.end();
      const exit = await program.exit;

      const refusals = {
        unknown: { GOODFETCH_HOEM: "/home/goodfetch" },
        empty: { GOODFETCH_URL: "" },
        channel: { GOODFETCH_CHANNEL: "nightly" },
        drainGrace: { GOODFETCH_DAEMON_DRAIN_GRACE: "5s" },
      };
      const refused = await Promise.all(
        Object.entries(refusals).map(async ([name, variable]) => {
          const home = join(testEnv.folder, name);
          const env = { ...scriptedEngineEnv(testEnv, "collie", SCRIPT, home), ...variable };
          // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
          const error = await failure(Collie.connect({ env }));
          return { name, error, started: scriptedEngineEnvironment(home) };
        }),
      );

      await clientCase.check("the engine was started with exactly the env map", () =>
        expect(asGiven(scriptedEngineEnvironment(givenHome), given)).toEqual(given),
      );
      await clientCase.check("the program with no env map ended with exit code 0", () =>
        // The received `stderr` shows in the diff when the program failed.
        expect({ exit, stderr: Buffer.concat(stderr).toString() }).toEqual({
          exit: { code: 0, signal: null },
          stderr: expect.any(String),
        }),
      );
      await clientCase.check(
        "with no env map, the engine was started with the process environment",
        () => {
          const own: unknown = JSON.parse(readFileSync(out, "utf8"));
          expect(own).toMatchObject({ [MARK]: "process", GOODFETCH_HOME: processHome });
          expect(scriptedEngineEnvironment(processHome)).toEqual(own);
        },
      );
      await clientCase.check(
        "a GOODFETCH_ variable off the list is setting_unknown, naming the closest",
        () =>
          expect(refused[0]?.error).toMatchObject({
            code: "setting_unknown",
            details: { key: "GOODFETCH_HOEM", closest: "GOODFETCH_HOME" },
          }),
      );
      await clientCase.check(
        "an empty value and one the engine cannot parse are setting_invalid",
        () =>
          expect(refused.slice(1).map(({ error }) => error)).toMatchObject([
            { code: "setting_invalid", details: { key: "GOODFETCH_URL" } },
            { code: "setting_invalid", details: { key: "GOODFETCH_CHANNEL" } },
            { code: "setting_invalid", details: { key: "GOODFETCH_DAEMON_DRAIN_GRACE" } },
          ]),
      );
      await clientCase.check("no refused connect() started an engine", () =>
        expect(refused.map(({ name, started }) => [name, started])).toEqual(
          Object.keys(refusals).map((name) => [name, undefined]),
        ),
      );
    },
  );
});
