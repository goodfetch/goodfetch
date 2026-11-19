// Which engine `connect()` reaches (ADRs 0024, 0078, 0083 and 0098), and
// the bytes to it when it is a Daemon. `engine` is `auto` unless code sets
// it: then the remote Daemon `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s
// token, else the Daemon of the SDK's line whose runtime file is in the
// Goodfetch home and answers, else an Attached engine. `attached` and
// `daemon` choose one of the two, and `{ url, token }` a remote Daemon,
// whatever the environment says; there is no `GOODFETCH_ENGINE`. A remote
// Daemon's address is `ws://` on loopback, reached with newline-delimited
// JSON-RPC over TCP, which an Exposed Daemon serves beside WebSocket, as
// the CLI reaches it (ADR 0024's note of 2026-10-23): `wss://` and
// addresses beyond loopback wait for the WebSocket transport and the
// Daemon's TLS, so a token never crosses the network in the clear. A token
// goes into `hello` alone, never into a log, an error or `EngineInfo`
// (ADR 0067).
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { connect, type Socket } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";
import { Duplex } from "node:stream";
import {
  DaemonUnreachableError,
  PRODUCT_VERSION,
  SettingInvalidError,
  type DaemonEndpoint,
} from "./generated/index.ts";
import type { Transport } from "./rpc.ts";

/** The variable naming a remote Daemon, which fills `engine` at `auto` alone (ADR 0083). */
const URL_VARIABLE = "GOODFETCH_URL";
/** The variable holding the remote Daemon's token. */
const TOKEN_VARIABLE = "GOODFETCH_TOKEN";
/** The variable naming the Goodfetch home. */
const HOME_VARIABLE = "GOODFETCH_HOME";

/** A remote Daemon: an Exposed Daemon or Goodfetch Cloud, and the token its `hello` takes. */
export interface RemoteEngine {
  /** Its address, `ws://` with a loopback IP address or `localhost` and a port, such as `ws://127.0.0.1:7878`. */
  readonly url: string;
  /** The token its `hello` takes, which nothing the SDK shows holds. */
  readonly token: string;
}

/**
 * The engine `connect()` reaches: `auto`, the default, takes the remote
 * Daemon `GOODFETCH_URL` names, else a running Daemon, else an Attached
 * engine; `attached` always starts an Attached engine; `daemon` reaches the
 * Daemon of the SDK's line, starting it when none runs; `{ url, token }`
 * reaches that remote Daemon.
 */
export type EngineChoice = "auto" | "attached" | "daemon" | RemoteEngine;

/** How the chosen engine is reached, before `hello`. */
export type Reach =
  | {
      readonly kind: "attached";
      /** The Goodfetch home the Attached engine runs in, or `null` when the OS names none. */
      readonly home: string | null;
    }
  | {
      readonly kind: "daemon" | "remote";
      /** The Daemon's home: `null` for a remote one, whose home is its own machine's. */
      readonly home: string | null;
      /** Where it listens: the remote Daemon's URL, or the Daemon's socket path or loopback address. */
      readonly url: string;
      /** The bytes to it, open. */
      readonly transport: Transport;
      /** The token its `hello` carries. */
      readonly token: string;
      /** What fails `connect()` when the Daemon closes the Connection before it answers `hello`. */
      readonly closedAtHello: () => DaemonUnreachableError;
    };

/** What choosing an engine needs. */
export interface EngineChoiceContext {
  /** The environment map, read here once. */
  readonly env: Readonly<Record<string, string>>;
  /** The executable a Daemon is started with, when `engine` is `daemon` and none runs. */
  readonly executable: () => string;
}

/**
 * Chooses the engine `choice` names, reading `GOODFETCH_URL` and
 * `GOODFETCH_TOKEN` at `auto` alone, and opens the bytes to a Daemon.
 *
 * @throws {SettingInvalidError} when a remote Daemon's URL names no loopback `ws://` address, or `GOODFETCH_URL` comes without `GOODFETCH_TOKEN`.
 * @throws {DaemonUnreachableError} when nothing answers where a remote Daemon's URL or a runtime file says, or the runtime file cannot be read.
 */
export async function reachEngine(
  choice: EngineChoice,
  context: EngineChoiceContext,
): Promise<Reach> {
  const { env } = context;
  if (typeof choice === "object") {
    return reachRemote(choice, "engine");
  }
  switch (choice) {
    case "attached":
      return { kind: "attached", home: homeOf(env) };
    case "daemon":
      return reachDaemon(env, context.executable);
    case "auto": {
      const remote = remoteOfEnvironment(env);
      if (remote !== undefined) {
        return reachRemote(remote, URL_VARIABLE);
      }
      const home = homeOf(env);
      const daemon = home === null ? undefined : await runningDaemon(home);
      return daemon ?? { kind: "attached", home };
    }
    default: {
      const unknown: never = choice;
      return unknown;
    }
  }
}

/** The remote Daemon `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s token; `undefined` when it is unset. */
function remoteOfEnvironment(env: Readonly<Record<string, string>>): RemoteEngine | undefined {
  const url = variable(env, URL_VARIABLE);
  if (url === undefined) {
    return undefined;
  }
  const token = variable(env, TOKEN_VARIABLE);
  if (token === undefined) {
    throw new SettingInvalidError(
      `${URL_VARIABLE} names a Daemon, and ${TOKEN_VARIABLE} holds no token for it`,
      {
        fix: `Set ${TOKEN_VARIABLE} to the token the Daemon ${URL_VARIABLE} names takes, or unset ${URL_VARIABLE} to use this machine's engine.`,
        details: {
          key: TOKEN_VARIABLE,
          reason: `${URL_VARIABLE} names a Daemon, and it holds no token`,
        },
      },
    );
  }
  return { url, token };
}

/**
 * Opens the bytes to the remote Daemon `remote` names, which `source`
 * (`engine` or `GOODFETCH_URL`) gave.
 */
async function reachRemote(remote: RemoteEngine, source: string): Promise<Reach> {
  const { url } = remote;
  const address = remoteAddress(url, source);
  const nothingAnswers = `Start the Daemon ${url} names, reach it through a forwarder that listens on loopback, such as an SSH tunnel, or leave engine at auto with ${URL_VARIABLE} unset to use this machine's engine.`;
  const socket = await open({ host: address.host, port: address.port });
  if (socket === undefined) {
    throw unreachable(`Nothing answers at ${url}, the Daemon ${source} names`, nothingAnswers);
  }
  return {
    kind: "remote",
    home: null,
    url,
    transport: transportOf(socket),
    token: remote.token,
    closedAtHello: () =>
      unreachable(
        `The Daemon at ${url} closed the Connection before it answered hello, as one does for a token it does not take`,
        source === URL_VARIABLE
          ? `Set ${TOKEN_VARIABLE} to the token the Daemon at ${url} takes.`
          : `Give engine the token the Daemon at ${url} takes.`,
      ),
  };
}

/**
 * The address a remote Daemon's URL names: `ws://` with a loopback IP
 * address or `localhost` and a port.
 *
 * @throws {SettingInvalidError} naming `source` otherwise.
 */
function remoteAddress(url: string, source: string): { host: string; port: number } {
  const refused = (why: string): SettingInvalidError =>
    new SettingInvalidError(`${source} ${url} ${why}`, {
      fix: `Give ${source} a Daemon's address on loopback, ws://<address>:<port>, such as ws://127.0.0.1:7878: a Daemon beyond loopback is reached through a forwarder that listens on it, such as an SSH tunnel, until the Daemon terminates TLS.`,
      details: { key: source, reason: `${url} ${why}` },
    });
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw refused("is no URL");
  }
  if (parsed.protocol !== "ws:") {
    throw refused("names no Daemon: give ws://<address>:<port>; wss:// waits for the Daemon's TLS");
  }
  if (parsed.port === "" || (parsed.pathname !== "/" && parsed.pathname !== "")) {
    throw refused("has no port, or has a path, where a Daemon's address has a port alone");
  }
  const host =
    parsed.hostname === "localhost" ? "127.0.0.1" : parsed.hostname.replace(/^\[|\]$/g, "");
  if (!isLoopback(host)) {
    throw refused(
      "names an address beyond loopback, where its token would cross the network in the clear until the Daemon terminates TLS",
    );
  }
  return { host, port: Number(parsed.port) };
}

/** Whether `host`, an IP address, is a loopback one. */
function isLoopback(host: string): boolean {
  return /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host) || host === "::1";
}

/**
 * The Daemon of the SDK's line, started with `daemon start` of the
 * Product's executable when none runs (ADR 0024).
 *
 * @throws {DaemonUnreachableError} when it cannot be started or reached.
 */
async function reachDaemon(
  env: Readonly<Record<string, string>>,
  executable: () => string,
): Promise<Reach> {
  const home = homeOf(env);
  if (home === null) {
    const fix = `Set ${HOME_VARIABLE} to the Goodfetch home, or a home folder the OS knows.`;
    throw new DaemonUnreachableError("No Goodfetch home names where the Daemon is found", {
      fix,
      details: { fix },
    });
  }
  const running = await runningDaemon(home);
  if (running !== undefined) {
    return running;
  }
  const path = executable();
  await startDaemon(path, env);
  const started = await runningDaemon(home);
  if (started === undefined) {
    const fix = `Start it with \`${path} daemon start\`, and find why it stopped in its log.`;
    throw new DaemonUnreachableError(`The Daemon ${path} started is not answering`, {
      fix,
      details: { fix },
    });
  }
  return started;
}

/** Runs `daemon start` of the executable at `path` with `env`, which returns once the Daemon serves. */
async function startDaemon(path: string, env: Readonly<Record<string, string>>): Promise<void> {
  const fix = `Start it with \`${path} daemon start\`, whose error says why it cannot, or leave engine at auto to use an Attached engine.`;
  const failed = (message: string, cause?: unknown): DaemonUnreachableError =>
    new DaemonUnreachableError(message, { fix, details: { fix }, cause });
  const child = spawn(path, ["daemon", "start", "--json"], {
    env: { ...env },
    stdio: ["ignore", "ignore", "ignore"],
    windowsHide: true,
  });
  const exit = await new Promise<number | null>((resolve, reject) => {
    child.once("error", (cause) =>
      reject(failed(`The Daemon could not be started with ${path}`, cause)),
    );
    child.once("close", (code) => resolve(code));
  });
  if (exit !== 0) {
    throw failed(`The Daemon could not be started: \`${path} daemon start\` exited with ${exit}`);
  }
}

/**
 * The Daemon whose runtime file is in `home`'s folder of the SDK's line,
 * when its endpoint answers; `undefined` when there is no runtime file, or
 * nothing listens where it says, as for a Daemon that died.
 *
 * @throws {DaemonUnreachableError} when the runtime file cannot be read.
 */
async function runningDaemon(home: string): Promise<Reach | undefined> {
  const file = join(home, lineOf(PRODUCT_VERSION), "daemon.json");
  let text: string;
  try {
    text = await readFile(file, "utf8");
  } catch (cause) {
    if (isCode(cause, "ENOENT")) {
      return undefined;
    }
    throw unreadable(file, cause);
  }
  const runtime = runtimeFileOf(text);
  if (runtime === undefined) {
    throw unreadable(file);
  }
  const { endpoint, token } = runtime;
  const url = "unix" in endpoint ? endpoint.unix : endpoint.tcp;
  const socket = await open("unix" in endpoint ? { path: endpoint.unix } : addressOf(endpoint.tcp));
  if (socket === undefined) {
    return undefined;
  }
  return {
    kind: "daemon",
    home,
    url,
    transport: transportOf(socket),
    token,
    closedAtHello: () => {
      const fix = `Restart it with \`daemon restart\`, which writes a new runtime file in ${home}.`;
      return new DaemonUnreachableError(
        `The Daemon at ${url} closed the Connection before it answered hello, as one does for a token it does not take`,
        { fix, details: { fix } },
      );
    },
  };
}

/** A Daemon nothing reaches: `daemon_unreachable`, its fix in its details too. */
function unreachable(message: string, fix: string): DaemonUnreachableError {
  return new DaemonUnreachableError(message, { fix, details: { fix } });
}

/** A runtime file that cannot be read fails `connect()`, as the CLI's commands do. */
function unreadable(file: string, cause?: unknown): DaemonUnreachableError {
  const fix = `Stop the Daemon with \`daemon stop\`, or remove ${file} when none runs, then connect again.`;
  return new DaemonUnreachableError(`The Daemon's runtime file ${file} cannot be read`, {
    fix,
    details: { fix },
    cause,
  });
}

/** The endpoint and token of a runtime file's text; `undefined` when it holds none. */
function runtimeFileOf(
  text: string,
): { readonly endpoint: DaemonEndpoint; readonly token: string } | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    !("endpoint" in parsed) ||
    !("token" in parsed)
  ) {
    return undefined;
  }
  const { endpoint, token } = parsed;
  if (typeof token !== "string" || typeof endpoint !== "object" || endpoint === null) {
    return undefined;
  }
  if ("unix" in endpoint && typeof endpoint.unix === "string") {
    return { endpoint: { unix: endpoint.unix }, token };
  }
  if ("tcp" in endpoint && typeof endpoint.tcp === "string") {
    return { endpoint: { tcp: endpoint.tcp }, token };
  }
  return undefined;
}

/** A loopback address `host:port`, as a runtime file names one. */
function addressOf(address: string): { host: string; port: number } {
  const end = address.lastIndexOf(":");
  return {
    host: address.slice(0, end).replace(/^\[|\]$/g, ""),
    port: Number(address.slice(end + 1)),
  };
}

/**
 * A connected socket to `where`; `undefined` when nothing listens there.
 *
 * @throws {DaemonUnreachableError} for any other error than nothing listening.
 */
async function open(
  where: { readonly path: string } | { readonly host: string; readonly port: number },
): Promise<Socket | undefined> {
  const socket = connect(where);
  try {
    await new Promise<void>((resolve, reject) => {
      socket.once("connect", resolve);
      socket.once("error", reject);
    });
  } catch (cause) {
    socket.destroy();
    if (isCode(cause, "ECONNREFUSED") || isCode(cause, "ENOENT")) {
      return undefined;
    }
    const fix =
      "Check that the Daemon's socket is yours and reachable, or connect with engine attached.";
    throw new DaemonUnreachableError("The Daemon's endpoint could not be opened", {
      fix,
      details: { fix },
      cause,
    });
  }
  return socket;
}

/** `socket` as a Connection's transport: its end is the Connection's end. */
function transportOf(socket: Socket): Transport {
  // A write after the Daemon closed fails the transport's writer, which
  // the Connection reports; the socket's own error event carries nothing more.
  socket.on("error", () => {});
  return Duplex.toWeb(socket) as Transport;
}

/**
 * The Goodfetch home: `GOODFETCH_HOME` when it is set, else the `goodfetch`
 * folder of the OS's usual data folder, as the engine finds it (`dirs`'s
 * `data_dir`); `null` when the environment names no home folder.
 */
export function homeOf(env: Readonly<Record<string, string>>): string | null {
  const named = variable(env, HOME_VARIABLE);
  if (named !== undefined) {
    return named;
  }
  const data = dataFolder(env);
  return data === null ? null : join(data, "goodfetch");
}

/** The OS's usual data folder for the user `env` is for. */
function dataFolder(env: Readonly<Record<string, string>>): string | null {
  if (process.platform === "win32") {
    return variable(env, "APPDATA") ?? null;
  }
  const home = variable(env, "HOME") ?? homedir();
  if (home === "") {
    return null;
  }
  if (process.platform === "darwin") {
    return join(home, "Library", "Application Support");
  }
  const xdg = variable(env, "XDG_DATA_HOME");
  return xdg !== undefined && xdg.startsWith("/") ? xdg : join(home, ".local", "share");
}

/**
 * The per-major folder's name of `version` (ADR 0030): its major, or
 * `0.y` during previews.
 */
export function lineOf(version: string): string {
  const [major = "0", minor = "0"] = version.split(/[.+-]/);
  return major === "0" ? `0.${minor}` : major;
}

/** The value of `name` in `env`; `undefined` when it is unset or empty, as the engine reads a deployment variable. */
function variable(env: Readonly<Record<string, string>>, name: string): string | undefined {
  const value = env[name];
  return value === undefined || value === "" ? undefined : value;
}

function isCode(error: unknown, code: string): boolean {
  return typeof error === "object" && error !== null && "code" in error && error.code === code;
}
