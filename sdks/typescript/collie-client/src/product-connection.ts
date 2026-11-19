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
import { ClientConnection, type EngineInfo } from "./client-connection.ts";
import { reachEngine, type EngineChoice, type Reach } from "./engine-choice.ts";
import type { ErrorClasses } from "./errors.ts";
import {
  EngineNotFoundError,
  SettingInvalidError,
  type Product,
  type RunLimit,
} from "./generated/index.ts";
import { settingsOnWire, type Settings } from "./generated/settings.ts";
import { timeoutOf } from "./handle.ts";
import { limitList, type Limits } from "./limits.ts";
import type { Logger } from "./logger.ts";
import { boundsOf, type ChannelBounds, type Transport } from "./rpc.ts";
import type { SettingsOnWire } from "./settings.ts";

/** Options of opening a Connection, over an Attached engine's stdio or a transport a test holds. */
export interface ConnectOverOptions {
  /**
   * Cancels the opening: it rejects with the signal's reason, once the
   * Attached engine it started has stopped and its streams are closed.
   */
  readonly signal?: AbortSignal | undefined;
  /**
   * How many calls may wait for their answer at once, 10,000 when left
   * out: past it a new call fails at once with `connection_overloaded`. A
   * whole number of 1 or more, as is the next bound, or the opening fails
   * with `setting_invalid`.
   */
  readonly maxPendingCalls?: ChannelBounds["maxPendingCalls"];
  /**
   * How many bytes may wait to be written to the engine, 64 MiB when left
   * out: past it a new call fails at once with `connection_overloaded`. One
   * message larger than this is still taken while nothing waits.
   */
  readonly maxQueuedWriteBytes?: ChannelBounds["maxQueuedWriteBytes"];
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
  readonly settings?: Settings | undefined;
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

/** Options of `connect()`. */
export interface ConnectOptions extends ConnectOverOptions {
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
   * gets exactly this map as its environment.
   */
  readonly env?: Readonly<Record<string, string>> | undefined;
}

/** What tells one Product's package from the other's when it connects. */
export interface ProductPackage {
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
  /** A module of the package, as `import.meta.url` gives it: where its platform package is resolved from. */
  readonly moduleUrl: string;
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
export function locateEngine(
  env: Readonly<Record<string, string>>,
  product: ProductPackage,
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
 */
export async function openEngine(
  product: ProductPackage,
  options: ConnectOptions,
): Promise<ClientConnection> {
  options.signal?.throwIfAborted();
  // Read before an engine starts, so a refused option leaves none behind.
  const common = opening(options);
  const env = options.env ?? processEnvironment();
  const reach = await reachEngine(options.engine ?? "auto", {
    env,
    executable: () => locateEngine(env, product),
    signal: options.signal,
  });
  const connection =
    reach.kind === "attached"
      ? await openAttached(product, env, reach.home, common)
      : await openDaemon(product, reach, common);
  const engine = connection.engine();
  options.logger?.info(reachedText(product, engine), { engine });
  return connection;
}

/** Starts `product`'s engine as an Attached engine with `env` and finishes `hello`. */
async function openAttached(
  product: ProductPackage,
  env: Readonly<Record<string, string>>,
  home: string | null,
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
    reached: { kind: "attached", home, url: null },
    needs: product.needs,
    errorClasses: product.errorClasses,
    ...common,
  });
}

/**
 * Finishes `hello` with a Daemon, its token in it: one that closes the
 * Connection before it answers fails with `daemon_unreachable`.
 */
function openDaemon(
  product: ProductPackage,
  reach: Exclude<Reach, { kind: "attached" }>,
  common: Opening,
): Promise<ClientConnection> {
  return ClientConnection.open({
    transport: reach.transport,
    reached: { kind: reach.kind, home: reach.home, url: reach.url },
    daemon: { token: reach.token, closedAtHello: reach.closedAtHello },
    needs: product.needs,
    errorClasses: product.errorClasses,
    ...common,
  });
}

/** The line logged once `connect()` reached `engine`. */
function reachedText(product: ProductPackage, engine: EngineInfo): string {
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
export function openOver(
  product: ProductPackage,
  transport: Transport,
  options: ConnectOverOptions,
): Promise<ClientConnection> {
  return ClientConnection.open({
    transport,
    needs: product.needs,
    errorClasses: product.errorClasses,
    ...opening(options),
  });
}

/** The common Connection's part of a Product's options. */
interface Opening {
  readonly bounds: ChannelBounds;
  readonly signal: AbortSignal | undefined;
  readonly limits: readonly RunLimit[];
  readonly settings: SettingsOnWire | undefined;
  readonly timeout: number | null | undefined;
  readonly logger: Logger | undefined;
}

/**
 * The common Connection's part of `options`.
 *
 * @throws {SettingInvalidError} when a bound is no whole number of 1 or more,
 * a Limit's amount does not turn exactly into the wire's, or the `timeout`
 * is 0, negative, fractional or not finite.
 */
function opening(options: ConnectOverOptions): Opening {
  const { signal, maxPendingCalls, maxQueuedWriteBytes, logger } = options;
  return {
    bounds: boundsOf({ maxPendingCalls, maxQueuedWriteBytes }),
    signal,
    limits: limitList(options.limits),
    settings: settingsOnWire(options.settings),
    timeout: timeoutOf(options.timeout, "timeout.connect"),
    logger,
  };
}

/** The process's own environment, its unset entries left out. */
function processEnvironment(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  );
}
