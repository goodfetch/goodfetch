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
import { userInfo } from "node:os";
import { join } from "node:path";
import { Duplex } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import {
  DaemonUnreachableError,
  EngineNotFoundError,
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
export interface RemoteDaemon {
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
export type EngineChoice = "auto" | "attached" | "daemon" | RemoteDaemon;

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
      /**
       * The text of the runtime file this machine's Daemon was found by,
       * which tells it from the Daemon it hands over to; `null` for a
       * remote one.
       */
      readonly runtimeFile: string | null;
    };

/** How a Daemon, this machine's or a remote one, is reached. */
export type DaemonReach = Exclude<Reach, { kind: "attached" }>;

/** What choosing an engine needs. */
export interface EngineChoiceContext {
  /** The environment map, read here once. */
  readonly env: Readonly<Record<string, string>>;
  /**
   * The executable a Daemon is started with, when `engine` is `daemon` and
   * none runs, and the fix `engine_not_found` gives when it is missing.
   */
  readonly executable: () => { readonly path: string; readonly notFoundFix: string };
  /** Cancels the choice: it rejects with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/**
 * Chooses the engine `choice` names, reading `GOODFETCH_URL` and
 * `GOODFETCH_TOKEN` at `auto` alone, and opens the bytes to a Daemon.
 *
 * @throws {SettingInvalidError} when a remote Daemon's URL names no loopback `ws://` address, or `GOODFETCH_URL` comes without `GOODFETCH_TOKEN`.
 * @throws {EngineNotFoundError} when `engine` is `daemon`, none runs, and its executable is missing.
 * @throws {DaemonUnreachableError} when nothing answers where a remote Daemon's URL says, or a Daemon cannot be started or its runtime file read.
 */
export async function reachEngine(
  choice: EngineChoice,
  context: EngineChoiceContext,
): Promise<Reach> {
  const { env, signal } = context;
  if (typeof choice === "object") {
    return reachRemote(choice, "engine", signal);
  }
  switch (choice) {
    case "attached":
      return { kind: "attached", home: homeOf(env) };
    case "daemon":
      return reachDaemon(context);
    case "auto": {
      const remote = remoteOfEnvironment(env);
      if (remote !== undefined) {
        return reachRemote(remote, URL_VARIABLE, signal);
      }
      const home = homeOf(env);
      const daemon = home === null ? undefined : await runningDaemon(home, signal);
      return daemon ?? { kind: "attached", home };
    }
    default: {
      const unknown: never = choice;
      return unknown;
    }
  }
}

/**
 * The remote Daemon `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s token;
 * `undefined` when it is unset. Its URL is judged first, as the CLI judges it.
 */
function remoteOfEnvironment(env: Readonly<Record<string, string>>): RemoteDaemon | undefined {
  const url = variable(env, URL_VARIABLE);
  if (url === undefined) {
    return undefined;
  }
  remoteAddress(url, URL_VARIABLE);
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
async function reachRemote(
  remote: RemoteDaemon,
  source: string,
  signal: AbortSignal | undefined,
): Promise<Reach> {
  const { url } = remote;
  const address = remoteAddress(url, source);
  const nothingAnswers = `Start the Daemon ${url} names, reach it through a forwarder that listens on loopback, such as an SSH tunnel, or leave engine at auto with ${URL_VARIABLE} unset to use this machine's engine.`;
  const socket = await open({ host: address.host, port: address.port }, signal);
  if (socket === undefined) {
    throw unreachable(`Nothing answers at ${url}, the Daemon ${source} names`, nothingAnswers);
  }
  return {
    kind: "remote",
    home: null,
    url,
    transport: transportOf(socket),
    token: remote.token,
    runtimeFile: null,
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
 * address or `localhost` and a port, and nothing else, so no credential in
 * it reaches a log, an error or `EngineInfo`.
 *
 * @throws {SettingInvalidError} naming `source` otherwise, without the URL
 * when it is no URL or holds more than an address, which may be a secret.
 */
function remoteAddress(url: string, source: string): { host: string; port: number } {
  const refused = (why: string, shown: string | null = url): SettingInvalidError => {
    const what = shown === null ? why : `${shown} ${why}`;
    return new SettingInvalidError(`${source} ${what}`, {
      fix: `Give ${source} a Daemon's address on loopback, ws://<address>:<port>, such as ws://127.0.0.1:7878: a Daemon beyond loopback is reached through a forwarder that listens on it, such as an SSH tunnel, until the Daemon terminates TLS.`,
      details: { key: source, reason: what },
    });
  };
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw refused("is no URL", null);
  }
  if (
    parsed.username !== "" ||
    parsed.password !== "" ||
    parsed.search !== "" ||
    parsed.hash !== ""
  ) {
    throw refused(
      "holds more than an address: a Daemon's address has no user, password, query or fragment, and its token goes in its own setting",
      null,
    );
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
async function reachDaemon(context: EngineChoiceContext): Promise<Reach> {
  const { env, signal } = context;
  const home = homeOf(env);
  if (home === null) {
    throw unreachable(
      "No Goodfetch home names where the Daemon is found",
      `Set ${HOME_VARIABLE} to the Goodfetch home, or a home folder the OS knows.`,
    );
  }
  const running = await runningDaemon(home, signal);
  if (running !== undefined) {
    return running;
  }
  const executable = context.executable();
  await startDaemon(executable, env, signal);
  const started = await runningDaemon(home, signal);
  if (started === undefined) {
    throw unreachable(
      `The Daemon ${executable.path} started is not answering`,
      `Start it with \`${executable.path} daemon start\`, and find why it stopped in its log.`,
    );
  }
  return started;
}

/**
 * Runs `daemon start --json` of `executable` with `env`, which returns once
 * the Daemon serves; a refused start fails with the message and fix it
 * printed.
 */
async function startDaemon(
  executable: { readonly path: string; readonly notFoundFix: string },
  env: Readonly<Record<string, string>>,
  signal: AbortSignal | undefined,
): Promise<void> {
  const { path } = executable;
  const child = spawn(path, ["daemon", "start", "--json"], {
    env: { ...env },
    stdio: ["ignore", "pipe", "ignore"],
    windowsHide: true,
    signal,
  });
  child.stdout.setEncoding("utf8");
  let printed = "";
  child.stdout.on("data", (text: string) => {
    printed += text;
  });
  const exit = await new Promise<number | null>((resolve, reject) => {
    child.once("error", (cause) => {
      if (signal?.aborted === true) {
        reject(signal.reason);
      } else if (isCode(cause, "ENOENT") || isCode(cause, "EACCES")) {
        reject(
          new EngineNotFoundError(`no engine executable at ${path}`, {
            fix: executable.notFoundFix,
            details: { path },
            cause,
          }),
        );
      } else {
        reject(
          unreachable(
            `The Daemon could not be started with ${path}`,
            `Start it with \`${path} daemon start\`, or leave engine at auto to use an Attached engine.`,
            cause,
          ),
        );
      }
    });
    child.once("close", (code) => resolve(code));
  });
  if (exit !== 0) {
    const refusal = refusalOf(printed);
    throw unreachable(
      `The Daemon could not be started: \`${path} daemon start\` exited with ${exit}${refusal === undefined ? "" : `: ${refusal.message}`}`,
      refusal?.fix ??
        `Start it with \`${path} daemon start\`, whose error says why it cannot, or leave engine at auto to use an Attached engine.`,
    );
  }
}

/** The message and fix of the `CommandFailed` a command printed under `--json`; `undefined` when it printed none. */
function refusalOf(
  printed: string,
): { readonly message: string; readonly fix: string | undefined } | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(printed);
  } catch {
    return undefined;
  }
  if (typeof parsed !== "object" || parsed === null || !("error" in parsed)) {
    return undefined;
  }
  const { error } = parsed;
  if (
    typeof error !== "object" ||
    error === null ||
    !("message" in error) ||
    typeof error.message !== "string"
  ) {
    return undefined;
  }
  return {
    message: error.message,
    fix: "fix" in error && typeof error.fix === "string" ? error.fix : undefined,
  };
}

/** How often the wait for the Daemon an idle one hands over to reads the runtime file. */
const HANDOVER_POLL = 50;

/**
 * The Daemon an idle Daemon of `home`, found by the runtime file whose text
 * is `handedOver`, hands over to once it refused `hello` with `handover`
 * (ADR 0098): its major's pick, which publishes its own runtime file once
 * it serves. A Node program cannot wait on the Daemon's start lock as the
 * CLI does, so the runtime file is read again every 50 ms; the one the
 * Daemon that handed over leaves until it stops is not taken.
 *
 * @returns `undefined` when none serves by `until`, a time as `Date.now()`
 * gives it; `null` waits with no end.
 * @throws {DaemonUnreachableError} when the runtime file cannot be read.
 * @throws the signal's reason once `signal` aborts.
 */
export async function handedOverTo(
  home: string,
  handedOver: string | null,
  until: number | null,
  signal: AbortSignal | undefined,
): Promise<DaemonReach | undefined> {
  for (;;) {
    const pick = await runningDaemon(home, signal, handedOver);
    if (pick !== undefined) {
      return pick;
    }
    if (until !== null && Date.now() >= until) {
      return undefined;
    }
    try {
      await delay(HANDOVER_POLL, undefined, { signal });
    } catch (cause) {
      // An aborted wait rejects with Node's own `AbortError`; `connect()` with the signal's reason.
      throw signal?.aborted === true ? signal.reason : cause;
    }
  }
}

/**
 * The Daemon whose runtime file is in `home`'s folder of the SDK's line,
 * when its endpoint answers; `undefined` when there is no runtime file, its
 * text is `skipped`, it holds no endpoint and token, as a dead Daemon's
 * half-written or older one, or nothing listens where it says, as for a
 * Daemon that died. A Node program cannot ask the Daemon's lock, so whether
 * the endpoint answers says whether it runs.
 *
 * @throws {DaemonUnreachableError} when the runtime file cannot be read.
 */
async function runningDaemon(
  home: string,
  signal: AbortSignal | undefined,
  skipped: string | null = null,
): Promise<DaemonReach | undefined> {
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
  if (text === skipped) {
    return undefined;
  }
  const runtime = runtimeFileOf(text);
  if (runtime === undefined) {
    return undefined;
  }
  const { endpoint, token } = runtime;
  const url = "unix" in endpoint ? endpoint.unix : endpoint.tcp;
  const socket = await open(
    "unix" in endpoint ? { path: endpoint.unix } : addressOf(endpoint.tcp),
    signal,
  );
  if (socket === undefined) {
    return undefined;
  }
  return {
    kind: "daemon",
    home,
    url,
    transport: transportOf(socket),
    token,
    runtimeFile: text,
    closedAtHello: () =>
      unreachable(
        `The Daemon at ${url} closed the Connection before it answered hello, as one does for a token it does not take`,
        `Restart it with \`daemon restart\`, which writes a new runtime file in ${home}.`,
      ),
  };
}

/** A Daemon nothing reaches: `daemon_unreachable`, its fix in its details too. */
function unreachable(message: string, fix: string, cause?: unknown): DaemonUnreachableError {
  return new DaemonUnreachableError(message, { fix, details: { fix }, cause });
}

/** A runtime file that cannot be read fails `connect()`, as the CLI's commands do. */
function unreadable(file: string, cause: unknown): DaemonUnreachableError {
  return unreachable(
    `The Daemon's runtime file ${file} cannot be read`,
    `Stop the Daemon with \`daemon stop\`, or remove ${file} when none runs, then connect again.`,
    cause,
  );
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
 * @throws the signal's reason once `signal` aborts.
 */
async function open(
  where: { readonly path: string } | { readonly host: string; readonly port: number },
  signal: AbortSignal | undefined,
): Promise<Socket | undefined> {
  signal?.throwIfAborted();
  const socket = connect(where);
  let aborted: (() => void) | undefined;
  try {
    await new Promise<void>((resolve, reject) => {
      socket.once("connect", resolve);
      socket.once("error", reject);
      aborted = () => reject(signal?.reason);
      signal?.addEventListener("abort", aborted, { once: true });
    });
  } catch (cause) {
    socket.destroy();
    if (signal?.aborted === true) {
      throw signal.reason;
    }
    if (isCode(cause, "ECONNREFUSED") || isCode(cause, "ENOENT")) {
      return undefined;
    }
    throw unreachable(
      "The Daemon's endpoint could not be opened",
      "Check that the Daemon's socket is yours and reachable, or connect with engine attached.",
      cause,
    );
  } finally {
    if (aborted !== undefined) {
      signal?.removeEventListener("abort", aborted);
    }
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
function homeOf(env: Readonly<Record<string, string>>): string | null {
  const named = variable(env, HOME_VARIABLE);
  if (named !== undefined) {
    return named;
  }
  const data = dataFolder(env);
  return data === null ? null : join(data, "goodfetch");
}

/**
 * The OS's usual data folder for the user `env` is for: its `HOME`, else
 * the user's home in the OS's user database, as `dirs` finds it, never the
 * process's own environment.
 */
function dataFolder(env: Readonly<Record<string, string>>): string | null {
  if (process.platform === "win32") {
    return variable(env, "APPDATA") ?? null;
  }
  const home = variable(env, "HOME") ?? userHome();
  if (home === undefined) {
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
function lineOf(version: string): string {
  const [major = "0", minor = "0"] = version.split(/[.+-]/);
  return major === "0" ? `0.${minor}` : major;
}

/** The user's home in the OS's user database; `undefined` when it has none. */
function userHome(): string | undefined {
  try {
    const { homedir } = userInfo();
    return homedir === "" ? undefined : homedir;
  } catch {
    return undefined;
  }
}

/** The value of `name` in `env`; `undefined` when it is unset or empty, as the engine reads a deployment variable. */
function variable(env: Readonly<Record<string, string>>, name: string): string | undefined {
  const value = env[name];
  return value === undefined || value === "" ? undefined : value;
}

function isCode(error: unknown, code: string): boolean {
  return typeof error === "object" && error !== null && "code" in error && error.code === code;
}
