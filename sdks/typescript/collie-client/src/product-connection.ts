// What a Product's package builds its Connection type on (ADRs 0024, 0078,
// 0090 and 0095): it reaches the engine `engine` chooses (`engine-choice`),
// a Daemon or, finding the Product's executable, the one its engine-path
// variable names or else its platform package's, an Attached engine started
// with exactly the program's environment map; finishes `hello`, naming the
// Product; and logs the engine reached at info. The other Product's
// variable is never read here.
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { AttachedEngine } from "./attached-engine.ts";
import {
  ClientConnection,
  connectLimit,
  phaseExceeded,
  type EngineInfo,
  type Reached,
} from "./client-connection.ts";
import { checkDeploymentVariables } from "./deployment-variables.ts";
import {
  handedOverTo,
  reachEngine,
  type DaemonReach,
  type EngineChoice,
  type LocalDaemonReach,
} from "./engine-choice.ts";
import type { ErrorClasses } from "./errors.ts";
import {
  isGoodfetchError,
  withFix,
  type GoodfetchError,
  type OperationNames,
} from "./goodfetch-error.ts";
import {
  EngineNotFoundError,
  PRODUCT_VERSION,
  SettingInvalidError,
  type Product,
  type RunLimit,
} from "./generated/index.ts";
import type { Settings } from "./generated/settings.ts";
import { timeoutOf } from "./handle.ts";
import { limitList, type Limits } from "./limits.ts";
import type { Logger } from "./logger.ts";
import { isRecord } from "./framing.ts";
import { LISTENER_QUEUE } from "./listeners.ts";
import {
  PENDING_CALLS,
  QUEUED_WRITE_BYTES,
  wholeBound,
  type ChannelBounds,
  type Transport,
} from "./rpc.ts";
import type { SettingsOnWire } from "./settings.ts";

/**
 * Options of opening a Connection, over an Attached engine's stdio or a
 * transport a test holds, with `S`, the settings its Product's `connect()`
 * takes: Collie's `Settings` by default.
 */
export interface ConnectOverOptions<S = Settings> {
  /**
   * Cancels the opening: it rejects with the signal's reason, once the
   * Attached engine it started has stopped and its streams are closed.
   */
  readonly signal?: AbortSignal | undefined;
  /**
   * The Limits of this Client connection (ADR 0080), set once `hello` is
   * answered and before the opening returns: each only lowers what covers
   * the connection's work. An amount the SDK cannot turn exactly fails the
   * opening with `setting_invalid` before the engine starts, and a Limit
   * the engine refuses closes the connection and fails the opening with
   * its error.
   */
  readonly limits?: Limits | undefined;
  /**
   * The Connection's own settings (ADR 0083), sent in `hello`, which every
   * Run it starts inherits: a key left out inherits configuration, `null`
   * clears a setting that can be cleared, and a value sets it. A Run's own
   * `settings` win over them key by key, and `timeout.run` is set here
   * only. A key the Connection does not take fails the opening with the
   * engine's error, such as `setting_not_applicable` or `setting_locked`.
   */
  readonly settings?: S | undefined;
  /**
   * How long the opening waits for the engine to answer `hello`, in
   * milliseconds (ADRs 0080 and 0083): `timeout.connect`'s 30 s when left
   * out, none for `null`. Past it, the opening fails with
   * `deadline_exceeded` naming `timeout.connect`; 0, a negative, a
   * fractional or a non-finite value fails it with `setting_invalid`
   * before the engine starts.
   */
  readonly timeout?: number | null | undefined;
  /**
   * The host's logger, which gets a start that resumed a Run at info and
   * one that found it ended at warning, an Attached engine's stderr lines
   * at warning, and the engine's log events at their level, `trace` at
   * `debug`. The engine sends those from the logger's `level` when it is
   * one of `off`, `error`, `warn`, `info`, `debug` and `trace`, else from
   * `info`, read at `connect()` and again when a Run starts. With none, the
   * SDK is silent and the engine sends no log events.
   */
  readonly logger?: Logger | undefined;
}

/** Options of `connect()`, with `S`, the settings it takes. */
export interface ConnectOptions<S = Settings> extends ConnectOverOptions<S> {
  /**
   * The engine to reach (ADR 0078): `auto` when left out, which takes the
   * remote Daemon `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s token, else
   * a Daemon running in the Goodfetch home, else an Attached engine;
   * `attached` starts an Attached engine; `daemon` reaches the Daemon,
   * starting it when none runs; `{ url, token }` reaches that remote Daemon,
   * `ws://` on loopback until the Daemon terminates TLS. A value here beats
   * `GOODFETCH_URL` (ADR 0083). `engine()` says which was reached.
   */
  readonly engine?: EngineChoice | undefined;
  /**
   * The environment map, read here and never at import: the process's own
   * when left out (ADRs 0062, 0083). Only the Product's own engine-path
   * variable is read (`GOODFETCH_COLLIE_ENGINE_PATH` by `Collie`,
   * `GOODFETCH_FOXHOUND_ENGINE_PATH` by `Foxhound`), and the Attached engine
   * gets exactly this map as its environment. A `GOODFETCH_` variable off
   * the closed list of deployment variables (`setting_unknown`, naming the
   * closest) or one set but empty (`setting_invalid`) fails `connect()`
   * before any engine is reached.
   */
  readonly env?: Readonly<Record<string, string>> | undefined;
}

/**
 * What tells one Product's package from the other's when it connects, with
 * `S`, the settings its `connect()` takes.
 */
export interface ProductPackage<S = Settings> {
  /** The Product its Client needs, which `hello` names. */
  readonly needs: Product;
  /** The Product's name in an error's message and `fix`, such as `Collie`. */
  readonly name: string;
  /** Its npm package, whose optional dependencies hold its engine, one platform package each. */
  readonly package: string;
  /** Its executable's name, without Windows's `.exe`. */
  readonly executable: string;
  /** Its engine-path variable, the only one read. */
  readonly enginePathVariable: string;
  /** The error class of each code the Product knows. */
  readonly errorClasses: ErrorClasses;
  /** Each of the Product's operations' names in TypeScript form, which heads the runtime's own errors (ADR 0079). */
  readonly operationNames: OperationNames;
  /** A module of the package, as `import.meta.url` gives it: where its platform package is resolved from. */
  readonly moduleUrl: string;
  /** Sends its `connect()`'s settings by their wire spelling, which `hello` carries. */
  readonly settingsOnWire: (settings: S | undefined) => SettingsOnWire | undefined;
}

/** Where to start a Product's engine, and what to do when it is not there. */
export interface EngineLocation {
  readonly path: string;
  /** The `fix` of `engine_not_found` when `path` is no executable. */
  readonly notFoundFix: string;
}

/**
 * The executable `env` names for `product`, or its platform package's.
 *
 * @throws {EngineNotFoundError} when the variable is unset and the platform package is not installed.
 * @throws {SettingInvalidError} when the variable is set but empty.
 */
export function locateEngine<S>(
  env: Readonly<Record<string, string>>,
  product: ProductPackage<S>,
): EngineLocation {
  const { name, executable, enginePathVariable: variable } = product;
  const named = env[variable];
  if (named === "") {
    // An empty value is never taken for unset (ADR 0083).
    throw new SettingInvalidError(`${variable} is set but empty`, {
      fix: `Set ${variable} to ${name}'s executable, ${executable}, or leave it unset to use the one ${product.package} installed.`,
      details: { key: variable, reason: "an empty value names no executable" },
    });
  }
  if (named !== undefined) {
    return {
      path: named,
      notFoundFix: `${variable} names ${named}, which is no executable file: point it at ${name}'s executable, ${executable}, or leave it unset to use the one ${product.package} installed.`,
    };
  }
  const platformPackage = `${product.package}-${process.platform}-${process.arch}`;
  const fix = `Install ${product.package} with its optional dependencies, which hold its engine in ${platformPackage}, or set ${variable} to ${name}'s executable.`;
  let manifest: string;
  try {
    manifest = createRequire(product.moduleUrl).resolve(`${platformPackage}/package.json`);
  } catch (cause) {
    throw new EngineNotFoundError(`${name}'s engine is not installed: no ${platformPackage}`, {
      fix,
      details: { package: platformPackage },
      cause,
    });
  }
  return {
    path: join(
      dirname(manifest),
      "bin",
      process.platform === "win32" ? `${executable}.exe` : executable,
    ),
    notFoundFix: fix,
  };
}

/**
 * Reaches the engine `options.engine` chooses for `product` and finishes
 * `hello`, naming the Product, so a missing executable (`engine_not_found`),
 * an unreachable Daemon (`daemon_unreachable`) or a refused Product
 * (`not_in_product`) or version fails here, with a `fix`, leaving no engine
 * behind; then logs the engine reached at info.
 *
 * This machine's Daemon that refuses `hello` by Product or version
 * (ADR 0098) is waited past when it hands over, until its major's pick
 * serves and answers `hello` within the same `timeout.connect`; when it
 * cannot hand over, `auto` runs an Attached engine, says why in
 * `engine().fallback` and warns once, and `daemon` fails with the refusal,
 * its fix naming this executable's `daemon restart`. A remote Daemon's
 * refusal fails unchanged.
 */
export async function openEngine<S>(
  product: ProductPackage<S>,
  options: ConnectOptions<S>,
): Promise<ClientConnection> {
  options.signal?.throwIfAborted();
  // Read before an engine starts, so a refused option leaves none behind.
  const common = opening(product, options);
  const env = options.env ?? processEnvironment();
  checkDeploymentVariables(env);
  const choice = options.engine ?? "auto";
  const reach = await reachEngine(choice, {
    env,
    executable: () => locateEngine(env, product),
    signal: options.signal,
  });
  const connection =
    reach.kind === "attached"
      ? await openAttached(product, env, { kind: "attached", home: reach.home, url: null }, common)
      : await openRefusable(product, env, choice === "auto", reach, common);
  const engine = connection.engine();
  options.logger?.info(reachedText(product, engine), { engine });
  return connection;
}

/**
 * Finishes `hello` with the Daemon `reach` opened, and with the pick it
 * hands over to, or runs an Attached engine in its place when `auto` and it
 * refuses by Product or version (ADR 0098).
 */
async function openRefusable<S>(
  product: ProductPackage<S>,
  env: Readonly<Record<string, string>>,
  auto: boolean,
  reach: DaemonReach,
  common: Opening,
): Promise<ClientConnection> {
  const startedAt = Date.now();
  let daemon = reach;
  for (;;) {
    try {
      return await openDaemon(product, daemon, common, startedAt);
    } catch (error) {
      if (daemon.kind === "remote" || !refusesByProductOrVersion(error)) {
        throw error;
      }
      const { home } = daemon;
      if (error.details["handover"] === true) {
        daemon = await pickOf(daemon, startedAt, common);
        continue;
      }
      const fix = `${error.fix ?? ""} ${refusalFix(product, env)}`.trim();
      if (!auto) {
        throw withFix(error, fix);
      }
      const fallback = {
        cause: `The Daemon at ${daemon.url} refused this Client with ${error.code}: ${error.message}`,
        fix,
      };
      common.logger?.warn(
        `${product.name} runs on an Attached engine instead of the Daemon at ${daemon.url}, which refused it with ${error.code}: ${error.message} ${fix}`,
        { fallback },
      );
      return openAttached(product, env, { kind: "attached", home, url: null, fallback }, common);
    }
  }
}

/** Whether `error` is a Daemon's refusal of `hello` by Product or protocol version. */
function refusesByProductOrVersion(error: unknown): error is GoodfetchError {
  return (
    isGoodfetchError(error) &&
    (error.code === "not_in_product" || error.code === "protocol_version_unsupported")
  );
}

/**
 * The pick the idle Daemon `handedOver` hands over to, once it serves
 * within `timeout.connect` of `startedAt`.
 *
 * @throws {DeadlineExceededError} naming `timeout.connect` and `waited_for` `hello` when none serves in time.
 */
async function pickOf(
  handedOver: LocalDaemonReach,
  startedAt: number,
  common: Opening,
): Promise<LocalDaemonReach> {
  const connect = connectLimit(common.timeout);
  if (connect === null) {
    return handedOverTo(handedOver, null, common.signal);
  }
  const pick = await handedOverTo(handedOver, startedAt + connect.value, common.signal);
  if (pick !== undefined) {
    return pick;
  }
  throw phaseExceeded(
    `timeout.connect (${connect.value} ms) passed before the Daemon an idle one handed over to served`,
    `Raise connect()'s timeout, or find why the Daemon did not start in its log in ${handedOver.home}.`,
    connect.limit,
    startedAt,
    "hello",
  );
}

/**
 * What a refusal's `fix` adds for `product`'s Client (ADR 0098), as the
 * engine words it: its executable's `daemon restart`, given by its path,
 * and Foxhound at this version or newer for Collie, whose executable is no
 * major's pick while a Foxhound of the major is installed. The SDK does not
 * read which executables are installed, so it names Foxhound for every
 * Collie Client, as the sentence's own condition says. With no executable
 * to name, the remedy is installing it, as `engine_not_found`'s fix says.
 */
function refusalFix<S>(product: ProductPackage<S>, env: Readonly<Record<string, string>>): string {
  let path: string;
  try {
    path = locateEngine(env, product).path;
  } catch (error) {
    if (error instanceof EngineNotFoundError && error.fix !== undefined) {
      return error.fix;
    }
    throw error;
  }
  const shown = /\s/.test(path) ? `"${path}"` : path;
  const restart = `Run \`${shown} daemon restart\` to restart the Daemon on its major's pick.`;
  return product.needs === "foxhound"
    ? restart
    : `${restart} ${product.name}'s executable would still not run the Daemon while a Foxhound of the major is installed: install Foxhound ${PRODUCT_VERSION} or newer (\`npm install @goodfetch/foxhound\`) and run its \`daemon restart\`.`;
}

/** Starts `product`'s engine as an Attached engine with `env` and finishes `hello`. */
async function openAttached<S>(
  product: ProductPackage<S>,
  env: Readonly<Record<string, string>>,
  reached: Reached,
  common: Opening,
): Promise<ClientConnection> {
  const location = locateEngine(env, product);
  const engine = await AttachedEngine.start({
    path: location.path,
    env,
    notFoundFix: location.notFoundFix,
    logger: common.logger,
  });
  return ClientConnection.open({
    transport: engine.transport,
    engine,
    reached,
    needs: product.needs,
    errorClasses: product.errorClasses,
    operationNames: product.operationNames,
    ...common,
  });
}

/**
 * Finishes `hello` with a Daemon, its token in it, within `timeout.connect`
 * of `connectStartedAt`: one that closes the Connection before it answers
 * fails with `daemon_unreachable`.
 */
function openDaemon<S>(
  product: ProductPackage<S>,
  reach: DaemonReach,
  common: Opening,
  connectStartedAt: number,
): Promise<ClientConnection> {
  return ClientConnection.open({
    transport: reach.transport,
    reached: { kind: reach.kind, home: reach.home, url: reach.url },
    daemon: { token: reach.token, closedAtHello: reach.closedAtHello },
    needs: product.needs,
    errorClasses: product.errorClasses,
    operationNames: product.operationNames,
    ...common,
    connectStartedAt,
  });
}

/** The line logged once `connect()` reached `engine`. */
function reachedText<S>(product: ProductPackage<S>, engine: EngineInfo): string {
  const version = `${engine.product} ${engine.version}`;
  switch (engine.kind) {
    case "attached":
      return `${product.name} connected to an Attached engine, ${version}, started from ${engine.enginePath ?? "its executable"}`;
    case "daemon":
      return `${product.name} connected to the Daemon at ${engine.url ?? "its endpoint"}, ${version}`;
    case "remote":
      return `${product.name} connected to the remote Daemon at ${engine.url ?? "its address"}, ${version}`;
    default: {
      const unknown: never = engine.kind;
      return unknown;
    }
  }
}

/**
 * Finishes `hello`, naming `product`, over `transport`, such as the
 * Scripted engine's in-memory player (ADR 0069): `connect()`'s Connection
 * with no process behind it.
 */
export function openOver<S>(
  product: ProductPackage<S>,
  transport: Transport,
  options: ConnectOverOptions<S>,
): Promise<ClientConnection> {
  return ClientConnection.open({
    transport,
    needs: product.needs,
    errorClasses: product.errorClasses,
    operationNames: product.operationNames,
    ...opening(product, options),
  });
}

/** The common Connection's part of a Product's options. */
interface Opening {
  readonly bounds: ChannelBounds;
  readonly listenerQueue: number;
  readonly signal: AbortSignal | undefined;
  readonly limits: readonly RunLimit[];
  readonly settings: SettingsOnWire | undefined;
  readonly timeout: number | null | undefined;
  readonly logger: Logger | undefined;
}

/**
 * The common Connection's part of `options`, their settings sent as
 * `product` spells them.
 *
 * @throws {SettingInvalidError} when a bound is no whole number of 1 or more,
 * a Limit's amount does not turn exactly into the wire's, or the `timeout`
 * is 0, negative, fractional or not finite.
 */
function opening<S>(product: ProductPackage<S>, options: ConnectOverOptions<S>): Opening {
  const { signal, logger } = options;
  const settings = product.settingsOnWire(options.settings);
  const own = clientBounds(settings);
  return {
    bounds: {
      maxPendingCalls: wholeBound(PENDING_CALLS, own["max_pending_calls"]),
      maxQueuedWriteBytes: wholeBound(QUEUED_WRITE_BYTES, own["max_queued_write_bytes"]),
    },
    listenerQueue: wholeBound(LISTENER_QUEUE, own["max_listener_queue"]),
    signal,
    limits: limitList(options.limits),
    settings,
    timeout: timeoutOf(options.timeout, "timeout.connect"),
    logger,
  };
}

/**
 * The `connection` area of the settings `hello` sends, where the Client's
 * own bounds are set (issue 2518): the engine checks and explains them as
 * any other setting, and the Client applies them to its own transport.
 */
function clientBounds(settings: SettingsOnWire | undefined): Readonly<Record<string, unknown>> {
  const area = settings?.["connection"];
  return isRecord(area) ? area : {};
}

/** The process's own environment, its unset entries left out. */
function processEnvironment(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  );
}
