// The project file `Collie.connect()` reads (ADR 0083; story 11-75), on the
// Scripted engine (ADR 0069): `project` names it, `engine()` reports it,
// `config.explain` and `run.start` carry it to the engine, whose script
// expects exactly those params, and its `timeout.connect` bounds `hello`
// unless the connection sets its own. A remote Daemon refuses a value the
// project file has no authority for there with `setting_locked`.
// `readProject`'s own tests cover how the file is found and refused.
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
    { tags: ["attached", "story:11-75", "operation:config.explain", "operation:run.start"] },
    async ({ testEnv }) => {
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

      expect([collie.engine().projectFile, report.lifecycle]).toEqual([project, "finished"]);
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
    { tags: ["attached", "story:11-75", "setting:timeout.connect", "error:deadline_exceeded"] },
    async ({ inFile, timeout, limit }, { testEnv }) => {
      const project = await writeProject(
        join(testEnv.folder, "work"),
        `version = "0.1"\n\n[settings.timeout]\nconnect = ${String(inFile)}\n`,
      );
      const env = scriptedEngineEnv(testEnv, "collie", "connect/hello-never-answered.json");

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, project, timeout }));

      expect(error).toMatchObject({
        code: "deadline_exceeded",
        limit,
        details: { waited_for: "hello" },
      });
    },
  );

  test(
    "is refused by a remote Daemon when a value widens tls.trusted_cas, with setting_locked",
    { tags: ["story:11-75", "error:setting_locked", "operation:run.start"] },
    async ({ testEnv, processes }) => {
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

      expect(await daemon.exit).toMatchObject({ code: 0 });
      expect(refused).toMatchObject({
        code: "setting_locked",
        details: { key: "tls.trusted_cas", layer: "project_configuration" },
      });
    },
  );
});
