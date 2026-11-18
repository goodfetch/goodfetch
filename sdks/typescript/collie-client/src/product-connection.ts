// What a Product's package builds its Connection type on (ADRs 0024, 0090
// and 0095): it finds the Product's executable, the one its engine-path
// variable names or else its platform package's, starts it as an Attached
// engine with exactly the program's environment map, and finishes `hello`,
// naming the Product. The other Product's variable is never read here.
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { AttachedEngine } from "./attached-engine.ts";
import { ClientConnection } from "./client-connection.ts";
import type { ErrorClasses } from "./errors.ts";
import {
  EngineNotFoundError,
  SettingInvalidError,
  type Product,
  type RunLimit,
} from "./generated/index.ts";
import { limitList, type Limits } from "./limits.ts";
import type { Logger } from "./logger.ts";
import { boundsOf, type ChannelBounds, type Transport } from "./rpc.ts";

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
}

/** Options of `connect()`. */
export interface ConnectOptions extends ConnectOverOptions {
  /**
   * The environment map, read here and never at import: the process's own
   * when left out (ADRs 0062, 0083). Only the Product's own engine-path
   * variable is read (`GOODFETCH_COLLIE_ENGINE_PATH` by `Collie`,
   * `GOODFETCH_FOXHOUND_ENGINE_PATH` by `Foxhound`), and the Attached engine
   * gets exactly this map as its environment.
   */
  readonly env?: Readonly<Record<string, string>> | undefined;
  /** The host's logger, which gets the engine's stderr lines at warning. With none, the SDK is silent. */
  readonly logger?: Logger | undefined;
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
 * Starts `product`'s engine as an Attached engine and finishes `hello`,
 * naming the Product, so a missing executable (`engine_not_found`) or a
 * refused Product (`not_in_product`) or version fails here, with a `fix`,
 * leaving no engine behind.
 */
export async function openAttached(
  product: ProductPackage,
  options: ConnectOptions,
): Promise<ClientConnection> {
  options.signal?.throwIfAborted();
  // Read before the engine starts, so a refused option leaves none behind.
  const common = opening(options);
  const env = options.env ?? processEnvironment();
  const location = locateEngine(env, product);
  const engine = await AttachedEngine.start({
    path: location.path,
    env,
    notFoundFix: location.notFoundFix,
    logger: options.logger,
  });
  return ClientConnection.open({
    transport: engine.transport,
    engine,
    needs: product.needs,
    errorClasses: product.errorClasses,
    ...common,
  });
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

/**
 * The common Connection's part of `options`.
 *
 * @throws {SettingInvalidError} when a bound is no whole number of 1 or more,
 * or a Limit's amount does not turn exactly into the wire's.
 */
function opening(options: ConnectOverOptions): {
  readonly bounds: ChannelBounds;
  readonly signal: AbortSignal | undefined;
  readonly limits: readonly RunLimit[];
} {
  const { signal, maxPendingCalls, maxQueuedWriteBytes } = options;
  return {
    bounds: boundsOf({ maxPendingCalls, maxQueuedWriteBytes }),
    signal,
    limits: limitList(options.limits),
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
