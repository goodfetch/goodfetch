// Which engine `Collie.connect()` reaches (ADRs 0024, 0078 and 0083): with
// `engine` left at `auto`, the remote Daemon `GOODFETCH_URL` names, else
// the Daemon whose runtime file is in the Goodfetch home, else an Attached
// engine; `attached`, `daemon` and `{ url, token }` choose one, beating
// `GOODFETCH_URL`. Each plays a Scripted engine's script (ADR 0069), as an
// Attached engine, as a Daemon on a Unix socket or on loopback TCP, or as
// a remote Daemon's WebSocket, and the Daemons of the same commit's
// executable. A Scripted Daemon exits 0
// only once every step played, so its exit says the Client sent exactly
// what the script holds, the Daemon's token in `hello` included.
import { Collie, type Logger } from "@goodfetch/collie";
import {
  failure,
  scriptedEngineEnv,
  searchCanaries,
  startScriptedDaemon,
  test,
  toolPath,
  type Canary,
} from "@goodfetch/test-support";
import { mkdir, utimes, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect } from "vitest";

/**
 * Writes a runtime file in `home`'s per-major folder, `0.0` for the
 * Scripted engine's version 0.0.0 (ADR 0030), as a Daemon publishes one.
 */
async function writeRuntimeFile(home: string, contents: string): Promise<void> {
  const folder = join(home, "0.0");
  await mkdir(folder, { recursive: true });
  await writeFile(join(folder, "daemon.json"), contents, { mode: 0o600 });
}

/** The token `connect/reached-remote-with-a-canary-token.json` takes: a planted canary (ADR 0067). */
const CANARY: Canary = {
  id: "remote-daemon-token",
  kind: "endpoint_token",
  value: "endpoint-token-canary-5f0e9b27c4d1",
};

/** One line the logger got. */
interface Logged {
  readonly level: "debug" | "info" | "warn" | "error";
  readonly message: string;
  readonly fields: Readonly<Record<string, unknown>> | undefined;
}

/** A host logger that keeps every line, asking for none of the engine's log events. */
function keepingLogger(): { readonly logger: Logger; readonly logged: Logged[] } {
  const logged: Logged[] = [];
  const keep =
    (level: Logged["level"]) =>
    (message: string, fields?: Readonly<Record<string, unknown>>): void => {
      logged.push({ level, message, fields });
    };
  return {
    logger: {
      level: "off",
      debug: keep("debug"),
      info: keep("info"),
      warn: keep("warn"),
      error: keep("error"),
    },
    logged,
  };
}

describe("Collie.connect() finds the engine", () => {
  test(
    "reaches an Attached engine when no Daemon runs, and says so",
    {
      tags: [
        "client_case/connect-finds-the-engine",
        "attached",
        "story:11-2",
        "story:11-4",
        "story:11-5",
        "story:11-8",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const { logger, logged } = keepingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const engine = collie.engine();
      await collie.close();

      await clientCase.check(
        "engine() reports an Attached engine, its executable and the home it runs in",
        () =>
          expect(engine).toEqual({
            kind: "attached",
            product: "collie",
            version: "0.0.0",
            protocolVersion: "0.1",
            projectFile: null,
            home: testEnv.home,
            enginePath: env["GOODFETCH_COLLIE_ENGINE_PATH"],
            url: null,
            fallback: null,
          }),
      );
      await clientCase.check("the choice is logged once at info, and nothing else", () =>
        expect(logged).toEqual([
          {
            level: "info",
            message: expect.stringContaining("Attached engine"),
            fields: { engine },
          },
        ]),
      );
    },
  );

  test(
    "reaches the Daemon whose runtime file is in the home, its token in hello, and says so",
    {
      tags: [
        "client_case/connect-finds-the-engine",
        "daemon",
        "story:11-2",
        "story:11-4",
        "story:11-5",
        "story:11-8",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-daemon.json",
        "socket",
      );
      const { logger, logged } = keepingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const engine = collie.engine();
      await collie.close();
      const exit = await daemon.exit;

      await clientCase.check(
        "the Daemon played its script through: hello carried its token, and the close ended the Connection",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
      await clientCase.check("engine() reports the Daemon, where it listens and its home", () =>
        expect(engine).toEqual({
          kind: "daemon",
          product: "collie",
          version: "0.0.0",
          protocolVersion: "0.1",
          projectFile: null,
          home: testEnv.home,
          enginePath: null,
          url: daemon.address,
          fallback: null,
        }),
      );
      await clientCase.check("the choice is logged once at info", () =>
        expect(logged).toEqual([
          { level: "info", message: expect.stringContaining("Daemon"), fields: { engine } },
        ]),
      );
    },
  );

  test(
    "reaches the remote Daemon GOODFETCH_URL names over a WebSocket of subprotocol goodfetch.v<major>, and its token shows nowhere",
    { tags: ["story:11-4", "story:11-5"] },
    async ({ testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-remote-with-a-canary-token.json",
        "websocket",
      );
      const url = `ws://${daemon.address}`;
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        GOODFETCH_URL: url,
        GOODFETCH_TOKEN: CANARY.value,
      };
      const { logger, logged } = keepingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const engine = collie.engine();
      await collie.close();

      expect(await daemon.exit).toMatchObject({ code: 0 });
      expect(engine).toEqual({
        kind: "remote",
        product: "collie",
        version: "0.0.0",
        protocolVersion: "0.1",
        projectFile: null,
        home: null,
        enginePath: null,
        url,
        fallback: null,
      });
      expect(
        searchCanaries(
          [CANARY],
          [
            { name: "engine()", text: JSON.stringify(engine) },
            { name: "logs", text: JSON.stringify(logged) },
          ],
        ),
      ).toEqual([]);
    },
  );

  test(
    "takes { url, token } written in code over GOODFETCH_URL",
    { tags: ["story:11-5"] },
    async ({ testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-remote-with-a-canary-token.json",
        "websocket",
      );
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        // Nothing listens here: reading it would fail the connect.
        GOODFETCH_URL: "ws://127.0.0.1:1",
        GOODFETCH_TOKEN: "not-the-token",
      };
      const url = `ws://localhost:${daemon.address.slice(daemon.address.lastIndexOf(":") + 1)}`;

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, engine: { url, token: CANARY.value } });
      const engine = collie.engine();
      await collie.close();

      expect(await daemon.exit).toMatchObject({ code: 0 });
      expect(engine).toMatchObject({ kind: "remote", url });
    },
  );

  test(
    "takes engine attached written in code over GOODFETCH_URL and a running Daemon",
    { tags: ["story:11-5"] },
    async ({ testEnv, processes }) => {
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        GOODFETCH_URL: "ws://127.0.0.1:1",
        GOODFETCH_TOKEN: CANARY.value,
      };
      // A running Daemon, which the Client must pass over.
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-daemon.json",
        "socket",
      );

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, engine: "attached" });
      const engine = collie.engine();
      await collie.close();
      const exit = await daemon.kill();

      expect(engine).toMatchObject({ kind: "attached", url: null });
      expect(exit).toMatchObject({ signal: "SIGKILL" });
    },
  );

  test(
    "fails with daemon_unreachable naming the fix when nothing answers at the url, the token in no message",
    { tags: ["story:11-5", "error:daemon_unreachable"] },
    async ({ testEnv }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const { logger, logged } = keepingLogger();

      const error = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({ env, logger, engine: { url: "ws://127.0.0.1:1", token: CANARY.value } }),
      );

      expect(error).toMatchObject({
        code: "daemon_unreachable",
        group: "unavailable",
        fix: expect.stringContaining("ws://127.0.0.1:1"),
      });
      expect(
        searchCanaries(
          [CANARY],
          [
            { name: "error", text: JSON.stringify(error, Object.getOwnPropertyNames(error)) },
            { name: "logs", text: JSON.stringify(logged) },
          ],
        ),
      ).toEqual([]);
    },
  );

  test(
    "fails with daemon_unreachable naming GOODFETCH_TOKEN when the Daemon closes the Connection at hello",
    { tags: ["story:11-5", "error:daemon_unreachable"] },
    async ({ testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-remote-with-a-canary-token.json",
        "websocket",
      );
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        GOODFETCH_URL: `ws://${daemon.address}`,
        GOODFETCH_TOKEN: "a-token-it-does-not-take",
      };

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env }));
      // The script expected another token: the Scripted engine fails its play.
      const exit = await daemon.exit;

      expect(exit.code).not.toBe(0);
      expect(error).toMatchObject({
        code: "daemon_unreachable",
        fix: expect.stringContaining("GOODFETCH_TOKEN"),
      });
      expect(JSON.stringify(error, Object.getOwnPropertyNames(error))).not.toContain(
        "a-token-it-does-not-take",
      );
    },
  );

  test(
    "fails with daemon_unreachable, never sending hello, when the Daemon answers no subprotocol this Client offers",
    { tags: ["story:11-5", "error:daemon_unreachable"] },
    async ({ testEnv, processes }) => {
      // A Daemon of another protocol major answers its own subprotocol only.
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-remote-with-a-canary-token.json",
        "websocket",
        testEnv.home,
        "goodfetch.v9999",
      );
      const url = `ws://${daemon.address}`;
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");

      const error = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({ env, engine: { url, token: CANARY.value } }),
      );
      // The script expected `hello`: the Scripted engine fails its play.
      const exit = await daemon.exit;

      expect(error).toMatchObject({
        code: "daemon_unreachable",
        group: "unavailable",
        fix: expect.stringContaining(url),
      });
      expect(exit.code).not.toBe(0);
    },
  );

  test(
    "reaches the Daemon whose runtime file names a loopback TCP address, as on Windows",
    { tags: ["story:11-4"] },
    async ({ testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-daemon.json",
        "tcp",
      );
      await writeRuntimeFile(
        testEnv.home,
        JSON.stringify({
          endpoint: { tcp: daemon.address },
          token: "scripted-daemon-token",
          version: "0.0.0",
          pid: 0,
        }),
      );
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const engine = collie.engine();
      await collie.close();

      expect(await daemon.exit).toMatchObject({ code: 0 });
      expect(engine).toMatchObject({
        kind: "daemon",
        home: testEnv.home,
        url: daemon.address,
      });
    },
  );

  test(
    "takes a runtime file that holds no endpoint for a dead Daemon's, and starts an Attached engine",
    { tags: ["story:11-4"] },
    async ({ testEnv }) => {
      await writeRuntimeFile(testEnv.home, "{ half-written");
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const engine = collie.engine();
      await collie.close();

      expect(engine).toMatchObject({ kind: "attached", url: null });
    },
  );

  test(
    "fails with engine_not_found naming the fix when engine daemon finds no Daemon and no executable",
    { tags: ["story:11-2", "error:engine_not_found"] },
    async ({ testEnv }) => {
      const missing = join(testEnv.home, "no-collie-here");
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        GOODFETCH_COLLIE_ENGINE_PATH: missing,
      };

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, engine: "daemon" }));

      expect(error).toMatchObject({
        code: "engine_not_found",
        details: { path: missing },
        fix: expect.stringContaining("GOODFETCH_COLLIE_ENGINE_PATH"),
      });
    },
  );

  test(
    "refuses a GOODFETCH_URL holding a password or a query, which shows in no message",
    { tags: ["story:11-5", "error:setting_invalid"] },
    async ({ testEnv }) => {
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        GOODFETCH_URL: `ws://user:${CANARY.value}@127.0.0.1:7878/?token=${CANARY.value}`,
        GOODFETCH_TOKEN: "t",
      };
      const { logger, logged } = keepingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, logger }));

      expect(error).toMatchObject({ code: "setting_invalid", details: { key: "GOODFETCH_URL" } });
      expect(
        searchCanaries(
          [CANARY],
          [
            { name: "error", text: JSON.stringify(error, Object.getOwnPropertyNames(error)) },
            { name: "logs", text: JSON.stringify(logged) },
          ],
        ),
      ).toEqual([]);
    },
  );

  test.for([
    {
      case: "names no loopback address",
      variables: { GOODFETCH_URL: "ws://192.0.2.1:7878", GOODFETCH_TOKEN: "t" },
      key: "GOODFETCH_URL",
    },
    {
      case: "is no ws:// address",
      variables: { GOODFETCH_URL: "https://127.0.0.1:7878", GOODFETCH_TOKEN: "t" },
      key: "GOODFETCH_URL",
    },
    {
      case: "comes without GOODFETCH_TOKEN",
      variables: { GOODFETCH_URL: "ws://127.0.0.1:7878" },
      key: "GOODFETCH_TOKEN",
    },
    {
      // The URL is judged first, as the CLI judges it.
      case: "is no ws:// address and comes without GOODFETCH_TOKEN",
      variables: { GOODFETCH_URL: "https://127.0.0.1:7878" },
      key: "GOODFETCH_URL",
    },
  ])(
    "fails with setting_invalid before any engine starts when GOODFETCH_URL $case",
    { tags: ["story:11-5", "error:setting_invalid"] },
    async ({ variables, key }, { testEnv }) => {
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        ...variables,
      };

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env }));

      expect(error).toMatchObject({ code: "setting_invalid", details: { key } });
    },
  );
});

/** The `fix` `connect/daemon-refused-protocol-version-unsupported.json`'s refusal gives. */
const VERSION_FIX =
  "Upgrade the engine to a version that speaks protocol 0.1, or use a Client of the engine's own version.";

/**
 * What a refusal's `fix` adds for a Collie Client whose executable is at
 * `path` and would be its major's pick (ADR 0098): its `daemon restart`.
 */
function restartFix(path: string): string {
  return `Run \`${path} daemon restart\` to restart the Daemon on its major's pick.`;
}

/** What a refusal's `fix` adds after the restart while a Foxhound of the major is installed (ADR 0098). */
const FOXHOUND_FIX =
  "Collie's executable would still not run the Daemon while a Foxhound of the major is installed: install Foxhound 0.0.0 or newer (`npm install @goodfetch/foxhound`) and run its `daemon restart`.";

/**
 * Records a Foxhound executable of the Scripted engine's version 0.0.0 in
 * `home`'s list of installed executables, at a file that is still at its
 * path, as its own run records it (ADR 0098).
 */
async function installFoxhound(home: string): Promise<void> {
  const path = join(home, "goodfetch-foxhound");
  const modifiedAt = new Date("2026-01-02T08:00:00Z");
  await writeFile(path, "foxhound");
  await utimes(path, modifiedAt, modifiedAt);
  const foxhound = {
    kind: "product_executable",
    product: "foxhound",
    product_version: "0.0.0",
    path,
    size: "foxhound".length,
    modified_at: "2026-01-02T08:00:00Z",
    pins: [],
    last_seen: new Date().toISOString(),
  };
  await writeFile(
    join(home, "installed-executables.json"),
    JSON.stringify({ format_version: "0.1", executables: [foxhound] }),
  );
}

describe("Collie.connect() when the Daemon refuses it by Product or protocol version", () => {
  test(
    "with engine auto, runs an Attached engine, says why in engine().fallback and warns once",
    {
      tags: [
        "client_case/connect-falls-back-when-the-daemon-refuses",
        "attached",
        "story:11-9",
        "story:11-290",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-refused-protocol-version-unsupported.json",
        "socket",
      );
      const { logger, logged } = keepingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const engine = collie.engine();
      await collie.close();
      const exit = await daemon.exit;

      await clientCase.check(
        "the Daemon played its script through: hello carried its token, and it refused it",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
      const fallback = {
        cause: expect.stringContaining("protocol_version_unsupported"),
        fix: `${VERSION_FIX} ${restartFix(env["GOODFETCH_COLLIE_ENGINE_PATH"] ?? "")}`,
      };
      await clientCase.check(
        "engine() reports the Attached engine, and the fallback's cause and fix",
        () =>
          expect(engine).toEqual({
            kind: "attached",
            product: "collie",
            version: "0.0.0",
            protocolVersion: "0.1",
            projectFile: null,
            home: testEnv.home,
            enginePath: env["GOODFETCH_COLLIE_ENGINE_PATH"],
            url: null,
            fallback,
          }),
      );
      await clientCase.check(
        "the fallback is logged once at warning, before the engine reached at info",
        () =>
          expect(logged).toEqual([
            {
              level: "warn",
              message: expect.stringContaining(daemon.address),
              fields: { fallback },
            },
            {
              level: "info",
              message: expect.stringContaining("Attached engine"),
              fields: { engine },
            },
          ]),
      );
    },
  );

  test(
    "with engine auto and no logger, falls back writing nothing to stdout or stderr",
    { tags: ["story:11-291"] },
    async ({ testEnv, processes }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-refused-protocol-version-unsupported.json",
        "socket",
      );
      const program = processes.spawn(
        process.execPath,
        [join(import.meta.dirname, "fixtures", "unlogged.ts")],
        { env },
      );
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      program.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
      program.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
      program.child.stdin.end();

      const exit = await program.exit;

      expect({
        daemon: await daemon.exit,
        exit,
        stdout: Buffer.concat(stdout).toString(),
        stderr: Buffer.concat(stderr).toString(),
      }).toEqual({
        daemon: expect.objectContaining({ code: 0 }),
        exit: { code: 0, signal: null },
        stdout: "",
        stderr: "",
      });
    },
  );

  test(
    "with engine daemon, fails with the refusal, whose fix adds this executable's daemon restart by its path",
    {
      tags: [
        "client_case/connect-falls-back-when-the-daemon-refuses",
        "daemon",
        "story:11-9",
        "story:11-293",
        "error:protocol_version_unsupported",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-refused-protocol-version-unsupported.json",
        "socket",
      );
      const { logger, logged } = keepingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, logger, engine: "daemon" }));
      const exit = await daemon.exit;

      await clientCase.check("the Daemon played its script through", () =>
        expect(exit).toMatchObject({ code: 0 }),
      );
      await clientCase.check(
        "connect() raises the refusal, its fix naming this executable's daemon restart alone, with no Foxhound installed",
        () =>
          expect(error).toMatchObject({
            name: "ProtocolVersionUnsupportedError",
            code: "protocol_version_unsupported",
            fix: `${VERSION_FIX} ${restartFix(env["GOODFETCH_COLLIE_ENGINE_PATH"] ?? "")}`,
            details: { client_protocol_version: "0.1", engine_protocol_version: "0.0" },
          }),
      );
      await clientCase.check("nothing is logged, and no Attached engine started", () =>
        expect(logged).toEqual([]),
      );
    },
  );

  test(
    "with engine daemon and a Foxhound of the major installed, the refusal's fix also names Foxhound at its version or newer",
    { tags: ["daemon", "story:11-293", "error:protocol_version_unsupported"] },
    async ({ testEnv, processes }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-refused-protocol-version-unsupported.json",
        "socket",
      );
      await installFoxhound(testEnv.home);

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, engine: "daemon" }));

      expect({ daemon: await daemon.exit, error }).toMatchObject({
        daemon: { code: 0 },
        error: {
          code: "protocol_version_unsupported",
          fix: `${VERSION_FIX} ${restartFix(env["GOODFETCH_COLLIE_ENGINE_PATH"] ?? "")} ${FOXHOUND_FIX}`,
        },
      });
    },
  );

  test.for([
    { way: "{ url, token } written in code", inEnv: false },
    { way: "GOODFETCH_URL", inEnv: true },
  ])(
    "fails with a remote Daemon's refusal through $way, which it never falls back from",
    { tags: ["story:11-295", "error:protocol_version_unsupported"] },
    async ({ inEnv }, { testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-refused-protocol-version-unsupported.json",
        "websocket",
      );
      const url = `ws://${daemon.address}`;
      const attached = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const env = inEnv
        ? { ...attached, GOODFETCH_URL: url, GOODFETCH_TOKEN: "scripted-daemon-token" }
        : attached;
      const { logger, logged } = keepingLogger();

      const error = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({
          env,
          logger,
          ...(inEnv ? {} : { engine: { url, token: "scripted-daemon-token" } }),
        }),
      );

      expect(await daemon.exit).toMatchObject({ code: 0 });
      expect(error).toMatchObject({ code: "protocol_version_unsupported", fix: VERSION_FIX });
      expect(logged).toEqual([]);
    },
  );

  test(
    "waits for the pick an idle Daemon hands over to, and reaches it",
    { tags: ["story:11-292"] },
    async ({ testEnv, processes }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const handing = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-hands-over.json",
        "socket",
      );
      const { logger, logged } = keepingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const connecting = Collie.connect({ env, logger });
      expect(await handing.exit).toMatchObject({ code: 0 });
      // The pick starts once the Daemon that handed over has stopped.
      const pick = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/reached-daemon.json",
        "socket",
      );
      const collie = await connecting;
      const engine = collie.engine();
      await collie.close();

      expect(await pick.exit).toMatchObject({ code: 0 });
      expect(engine).toMatchObject({ kind: "daemon", url: pick.address, fallback: null });
      expect(logged).toEqual([
        { level: "info", message: expect.stringContaining("Daemon"), fields: { engine } },
      ]);
    },
  );

  test(
    "fails with deadline_exceeded naming timeout.connect when no pick follows a handover in time",
    { tags: ["story:11-292", "error:deadline_exceeded"] },
    async ({ testEnv, processes }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const handing = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-hands-over.json",
        "socket",
      );

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const error = await failure(Collie.connect({ env, timeout: 300 }));

      expect(await handing.exit).toMatchObject({ code: 0 });
      expect(error).toMatchObject({
        code: "deadline_exceeded",
        limit: { key: "timeout.connect", value: 300, layer: "run", origin: "argument" },
        details: { waited_for: "hello" },
      });
    },
  );
});

describe("Collie.connect() and the Daemon of the same commit", () => {
  test.override("withDaemon", true);

  test(
    "reaches the running Daemon with engine left at auto, and an Attached engine when asked",
    { tags: ["story:11-2", "story:11-4"] },
    async ({ env, daemon, testEnv }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const onDaemon = await Collie.connect({ env });
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const attached = await Collie.connect({ env, engine: "attached" });
      const engines = [onDaemon.engine(), attached.engine()];
      await Promise.all([onDaemon.close(), attached.close()]);

      expect(engines).toEqual([
        expect.objectContaining({
          kind: "daemon",
          product: "collie",
          home: testEnv.home,
          enginePath: null,
          url: daemon.socket,
        }),
        expect.objectContaining({
          kind: "attached",
          enginePath: env["GOODFETCH_COLLIE_ENGINE_PATH"],
          url: null,
        }),
      ]);
    },
  );

  test(
    "takes an environment whose only engine-path variable is Foxhound's, and reaches the Daemon",
    { tags: ["story:11-299"] },
    async ({ env, daemon }) => {
      const foxhoundsOnly = {
        ...Object.fromEntries(
          Object.entries(env).filter(([name]) => name !== "GOODFETCH_COLLIE_ENGINE_PATH"),
        ),
        GOODFETCH_FOXHOUND_ENGINE_PATH: "/nonexistent/goodfetch-foxhound",
      };

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: foxhoundsOnly });
      const engine = collie.engine();
      await collie.close();

      expect(engine).toMatchObject({ kind: "daemon", enginePath: null, url: daemon.socket });
    },
  );
});

describe("Collie.connect() with engine daemon", () => {
  test(
    "starts the Daemon when none runs, and reaches it",
    { tags: ["story:11-4"] },
    async ({ env, testEnv, processes }) => {
      const executable = toolPath(testEnv.tools, "collie");
      try {
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        const collie = await Collie.connect({ env, engine: "daemon" });
        const engine = collie.engine();
        await collie.close();

        expect(engine).toMatchObject({ kind: "daemon", product: "collie", home: testEnv.home });
      } finally {
        const stop = processes.spawn(executable, ["daemon", "stop"], { env });
        stop.child.stdout.resume();
        stop.child.stderr.resume();
        expect(await stop.exit).toMatchObject({ code: 0 });
      }
    },
  );
});
