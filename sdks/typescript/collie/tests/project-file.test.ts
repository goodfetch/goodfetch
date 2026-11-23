// The project file `Collie.connect()` reads (ADR 0083; story 11-75): the
// `connect-reads-the-project-file` Client conformance case, on the Scripted
// engine (ADR 0069). By default the file is found upward from the working
// directory, shown in a program the test starts below it, so no test moves
// its own; `project` names another and `null` reads none. `engine()`
// reports it, and `config.explain` and `run.start` carry it to the engine,
// whose script expects exactly those params. Its `timeout.connect` bounds
// `hello` unless the connection sets its own. A remote Daemon refuses a
// value the project file has no authority for there with `setting_locked`.
// `readProject`'s own tests cover how the file is refused. Python's
// `test_project_file.py` is its twin.
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Collie } from "@goodfetch/collie";
import {
  failure,
  projectScript,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The remote Daemon's token in `connect/project-value-locked-by-a-remote-daemon.json`. */
const TOKEN = "endpoint-token-canary-5f0e9b27c4d1";

/** Writes `text` as the project file `goodfetch.toml` in `folder`, creating it, and returns its path. */
async function writeProject(folder: string, text: string): Promise<string> {
  await mkdir(folder, { recursive: true });
  const file = join(folder, "goodfetch.toml");
  await writeFile(file, text);
  return file;
}

describe("The project file Collie.connect() reads", () => {
  test(
    "reaches the engine on config.explain and run.start, and engine() reports it",
    {
      tags: [
        "client_case/connect-reads-the-project-file",
        "attached",
        "story:11-75",
        "operation:config.explain",
        "operation:run.start",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const project = await writeProject(
        join(testEnv.folder, "work"),
        'version = "0.1"\n\n[settings.timeout]\nnavigation = 45000\n',
      );
      const env = scriptedEngineEnv(
        testEnv,
        "collie",
        projectScript(testEnv, "run/project-file-carried.json", project),
      );

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, project });
      // The script expects each call's params, so it plays to its end only
      // when both carry the file read.
      await collie.explainSettings(["timeout.navigation"]);
      const report = await (await collie.start()).finish();
      await collie.close();

      await clientCase.check(
        "the file project names reached the engine on config.explain and run.start, and the engine info reports it",
        () =>
          expect([collie.engine().projectFile, report.lifecycle]).toEqual([project, "finished"]),
      );
    },
  );

  test(
    "is found upward from the working directory, and null reads none",
    {
      tags: [
        "client_case/connect-reads-the-project-file",
        "attached",
        "story:11-75",
        "operation:config.explain",
        "operation:run.start",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const project = realpathSync(
        await writeProject(
          join(testEnv.folder, "project"),
          'version = "0.1"\n\n[settings.timeout]\nnavigation = 45000\n',
        ),
      );
      const deeper = join(testEnv.folder, "project", "src", "deeper");
      await mkdir(deeper, { recursive: true });
      const scripts = {
        found: projectScript(testEnv, "run/project-file-carried.json", project),
        none: "run/project-file-read-none.json",
      };

      // Each program runs alone: the second starts once the first has ended.
      const seen: Record<string, unknown> = {};
      for (const [chosen, script] of Object.entries(scripts)) {
        const out = join(testEnv.folder, `${chosen}.json`);
        const program = processes.spawn(
          process.execPath,
          [join(import.meta.dirname, "fixtures", "project.ts"), out, chosen],
          {
            env: scriptedEngineEnv(testEnv, "collie", script, join(testEnv.folder, chosen)),
            cwd: deeper,
          },
        );
        const stderr: Buffer[] = [];
        program.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
        program.child.stdin.end();
        const exit = await program.exit;
        seen[chosen] = {
          exit,
          // The received `stderr` shows in the diff when the program failed.
          stderr: Buffer.concat(stderr).toString(),
          played: existsSync(out) ? (JSON.parse(readFileSync(out, "utf8")) as unknown) : undefined,
        };
      }

      await clientCase.check(
        "with no project, the file found upward from the working directory reached the engine, and the engine info reports it",
        () =>
          expect(seen["found"]).toEqual({
            exit: { code: 0, signal: null },
            stderr: expect.any(String),
            played: { projectFile: project, lifecycle: "finished" },
          }),
      );
      await clientCase.check("with project null, none reached the engine", () =>
        expect(seen["none"]).toEqual({
          exit: { code: 0, signal: null },
          stderr: expect.any(String),
          played: { projectFile: null, lifecycle: "finished" },
        }),
      );
    },
  );

  test.for([
    {
      name: "bounds hello with its timeout.connect, naming the project file's layer",
      inFile: 200,
      timeout: undefined,
      limit: { key: "timeout.connect", value: 200, layer: "project_configuration" },
    },
    {
      name: "bounds hello with the connection's own timeout over its timeout.connect",
      inFile: 60_000,
      timeout: 200,
      limit: { key: "timeout.connect", value: 200, layer: "run", origin: "argument" },
    },
  ])(
    "$name",
    {
      tags: [
        "client_case/connect-reads-the-project-file",
        "attached",
        "story:11-75",
        "setting:timeout.connect",
        "error:deadline_exceeded",
      ],
    },
    async ({ inFile, timeout, limit }, { testEnv, clientCase }) => {
      const project = await writeProject(
        join(testEnv.folder, "work"),
        `version = "0.1"\n\n[settings.timeout]\nconnect = ${String(inFile)}\n`,
      );
      const env = scriptedEngineEnv(testEnv, "collie", "connect/hello-never-answered.json");

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, project, timeout }));

      await clientCase.check(
        "hello unanswered failed with deadline_exceeded, its limit naming the timeout.connect that bounded it",
        () =>
          expect(error).toMatchObject({
            code: "deadline_exceeded",
            limit,
            details: { waited_for: "hello" },
          }),
      );
    },
  );

  test(
    "is refused by a remote Daemon when a value widens tls.trusted_cas, with setting_locked",
    {
      tags: [
        "client_case/connect-reads-the-project-file",
        "daemon",
        "story:11-75",
        "error:setting_locked",
        "operation:run.start",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const project = await writeProject(
        join(testEnv.folder, "work"),
        'version = "0.1"\n\n[settings.tls.trusted_cas]\n"intranet.example" = ["/etc/ssl/intranet-ca.pem"]\n',
      );
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        projectScript(testEnv, "connect/project-value-locked-by-a-remote-daemon.json", project),
        "websocket",
      );
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({
        env,
        project,
        engine: { url: `ws://${daemon.address}`, token: TOKEN },
      });
      const refused = await failure(collie.start());
      await collie.close();

      await clientCase.check(
        "the remote Daemon read the project file on run.start and played its script",
        async () => expect(await daemon.exit).toMatchObject({ code: 0 }),
      );
      await clientCase.check(
        "the Run's start raised setting_locked on tls.trusted_cas at project_configuration",
        () =>
          expect(refused).toMatchObject({
            code: "setting_locked",
            details: { key: "tls.trusted_cas", layer: "project_configuration" },
          }),
      );
    },
  );
});
