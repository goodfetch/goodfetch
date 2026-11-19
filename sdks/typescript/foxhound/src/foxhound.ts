// Foxhound's Connection type (ADRs 0078, 0090 and 0095): `Foxhound.connect()`
// finds Foxhound's executable, starts it as an Attached engine with exactly
// the program's environment map, and finishes `hello`, naming Foxhound,
// before it returns, so a Collie engine refuses it there with
// `not_in_product`. `startCrawl` starts a crawl from a `Crawl` value and
// serves the engine's `crawl_request.handle` requests to its handlers.
// Everything else is the client package's common Connection, whose members
// `Collie` carries under the same names.
import type {
  ConnectOptions,
  ConnectOverOptions,
  Connection,
  EngineInfo,
  JSONValue,
  Limits,
  Money,
  RawCallOptions,
  RawParams,
  Run,
  StartOptions,
} from "@goodfetch/collie-client";
import { RunNotFoundError } from "@goodfetch/collie-client";
import {
  holdInRealm,
  openEngine,
  openOver,
  type ClientConnection,
  type Transport,
  warnEntry,
} from "@goodfetch/collie-client/runtime";
import { checkCrawlStartable, crawlDeclaration, type Crawl } from "./crawl.ts";
import { CrawlRun, handleRequest, type CrawlHost } from "./crawl-run.ts";
import {
  FoxhoundOperations,
  foxhoundOperations,
  type CrawlRequestHandleParams,
  type CrawlRequestHandled,
  type CrawlStartParams,
  type FoxhoundOperationTypes,
  type RequestSeed,
} from "./generated/index.ts";
import { settingsOnWire, type CrawlSettings } from "./generated/settings.ts";
import { foxhoundTimes } from "./generated/times.ts";
import { foxhound } from "./product.ts";

/**
 * Foxhound's Limits (ADR 0080): Collie's, and Paid fetches, which only
 * Foxhound makes. A Paid fetch Limit set in code only caps them: a Paid
 * fetch is made only under a Limit a Client could not set (ADRs 0041, 0045).
 */
export interface FoxhoundLimits extends Limits {
  /** Paid fetches. */
  readonly paidFetches?: number | undefined;
  /** What each Usage kind may cost, in one currency. */
  readonly price?:
    | (NonNullable<Limits["price"]> & { readonly paidFetches?: Money | undefined })
    | undefined;
}

/** Options of `connectOver()`, with Foxhound's Limits. */
export interface FoxhoundConnectOverOptions extends ConnectOverOptions {
  readonly limits?: FoxhoundLimits | undefined;
}

/** Options of `connect()`, with Foxhound's Limits. */
export interface FoxhoundConnectOptions extends ConnectOptions {
  readonly limits?: FoxhoundLimits | undefined;
}

/** What starting a Run takes, with Foxhound's Limits. */
export interface FoxhoundStartOptions extends StartOptions {
  readonly limits?: FoxhoundLimits | undefined;
}

/** What starting a crawl takes (ADR 0093). */
export interface StartCrawlOptions {
  /** The requests the crawl starts from: at least one. */
  readonly seeds: readonly [RequestSeed, ...RequestSeed[]];
  /** The Run's ID; the engine generates one when it is left out. An interrupted Run's ID resumes it; a finished one's returns it as it ended. */
  readonly runId?: string | undefined;
  /** Its tags: at most 16, each at most 64 characters (ADR 0036). */
  readonly tags?: readonly string[] | undefined;
  /**
   * Its settings, Collie's and Foxhound's, nested by camelCase segment,
   * such as `{ crawl: { handler: { concurrency: 4 } } }` (ADRs 0083 and
   * 0093).
   */
  readonly settings?: CrawlSettings | undefined;
}

let open: (connection: ClientConnection) => Foxhound;

/**
 * A Connection to a Foxhound engine (ADR 0095): the entry point (ADR 0078).
 * Each one is independent, and many may be open in one process. It belongs
 * to the realm that opened it: a `worker_threads` worker connects on its
 * own (ADR 0081). A helper typed for `Connection` takes it as it takes a
 * `Collie`.
 */
export class Foxhound implements Connection {
  static {
    open = (connection) => new Foxhound(connection);
  }

  readonly #connection: ClientConnection;
  readonly #operations: FoxhoundOperations;
  readonly #crawlHost: CrawlHost;
  /** The crawls' Runs started on this Connection and not closed, by Run ID. */
  readonly #crawlRuns = new Map<string, CrawlRun>();
  /** The crawl starts not yet answered, whose work items may come before their answer is read. */
  readonly #starting = new Set<Promise<CrawlRun>>();

  private constructor(connection: ClientConnection) {
    this.#connection = connection;
    const calls = connection.caller<FoxhoundOperationTypes>(foxhoundTimes);
    this.#operations = new FoxhoundOperations(calls);
    this.#crawlHost = {
      connection,
      calls,
      forget: (run) => {
        // A later start of the same Run ID holds its own entry.
        if (this.#crawlRuns.get(run.id()) === run) this.#crawlRuns.delete(run.id());
      },
    };
    connection.serve("crawl_request.handle", async (params, signal) =>
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the engine's requests are not validated beyond `hello`'s protocol version (STYLE.md): `crawl_request.handle` carries its params as the catalog types them.
      this.#handle(params as CrawlRequestHandleParams, signal),
    );
    holdInRealm(this);
  }

  /**
   * Starts Foxhound's engine as an Attached engine and completes `hello`
   * before returning, so a missing executable (`engine_not_found`), a
   * Collie engine (`not_in_product`, whose details name the engine's
   * Product and version and whose `fix` is the install command) or a
   * refused version fails here, before any other call. An aborted `signal`
   * fails it with the signal's reason, leaving no engine behind.
   */
  static async connect(options: FoxhoundConnectOptions = {}): Promise<Foxhound> {
    return open(await openEngine(foxhound, options));
  }

  /** The engine reached: its kind, Product, Product version and protocol version. Read without a round trip. */
  engine(): EngineInfo {
    return this.#connection.engine();
  }

  /**
   * Starts a Run and returns once it is started (ADR 0102); a `runId`
   * naming an interrupted Run resumes it, and one naming a finished Run
   * returns it as it ended. `finish()` ends it; closing it unfinished stops
   * it, resumable by its Run ID.
   */
  start(options?: FoxhoundStartOptions): Promise<Run> {
    return this.#connection.start(options);
  }

  /**
   * Starts `crawl` from its seeds and returns once its Run is started (ADR
   * 0093). The engine then leases each work item to the handler of its
   * Label, on this process's event loop, and the Run finishes once every
   * work item is done. A `runId` naming an interrupted Run resumes it, and
   * one naming a finished Run returns it as it ended.
   * Before anything is sent, a Label without a handler fails with
   * `label_unhandled`, and a seed whose Label's name `crawl` does not
   * declare with `label_unknown` (a seed carries its Label's name only).
   */
  async startCrawl(crawl: Crawl, options: StartCrawlOptions): Promise<CrawlRun> {
    warnEntry(foxhoundOperations, "crawl.start");
    checkCrawlStartable(crawl, options.seeds);
    const settings = settingsOnWire(options.settings);
    const params: CrawlStartParams = {
      ...crawlDeclaration(crawl),
      seeds: [...options.seeds],
      ...(options.runId === undefined ? {} : { run_id: options.runId }),
      ...(options.tags === undefined ? {} : { tags: [...options.tags] }),
      ...(settings === undefined ? {} : { settings }),
    };
    const starting = this.#connection.startRun(
      "crawl.start",
      () => this.#operations.crawlStart(params),
      (host, started) => {
        const run = new CrawlRun(host, started, crawl, this.#crawlHost);
        this.#crawlRuns.set(run.id(), run);
        return run;
      },
    );
    this.#starting.add(starting);
    try {
      return await starting;
    } finally {
      this.#starting.delete(starting);
    }
  }

  /**
   * Sends `operation`, one the SDK may not generate that acts on no value
   * or whose `params` name every value, with `params` as the wire's JSON in
   * snake_case, and answers its result as the engine sent it (ADR 0099).
   * `timeout` is sent as the operation's own; `signal` cancels the call. It
   * fails with the same classes as any call, `closed` once the Connection
   * is closed, and is never retried.
   */
  call(operation: string, params?: RawParams, options?: RawCallOptions): Promise<JSONValue> {
    return this.#connection.call(operation, params, options);
  }

  /**
   * Closes the Connection; idempotent. Closes every Run it opened first,
   * stopping one still running, cancels the calls in flight, then stops the
   * Attached engine and waits for it. Any later call fails with `closed`.
   */
  close(): Promise<void> {
    return this.#connection.close();
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  /** Answers a `crawl_request.handle` request with the handler of its crawl's Run. */
  async #handle(
    request: CrawlRequestHandleParams,
    signal: AbortSignal,
  ): Promise<CrawlRequestHandled> {
    if (!this.#crawlRuns.has(request.run_id)) {
      await Promise.allSettled(this.#starting);
    }
    const run = this.#crawlRuns.get(request.run_id);
    if (run === undefined) {
      throw new RunNotFoundError(
        `the crawl request ${request.crawl_request.id} names the Run ${request.run_id}, which no crawl started on this Connection runs`,
        { details: { run_id: request.run_id } },
      );
    }
    return handleRequest(run, request, signal);
  }
}

/**
 * Opens a Foxhound Connection over `transport`, such as the Scripted
 * engine's in-memory player in a program's tests (ADR 0069): the same
 * Connection as `connect()`'s, with no process behind it. Programs reach it
 * through `@goodfetch/foxhound/testing`.
 */
export async function connectOver(
  transport: Transport,
  options: FoxhoundConnectOverOptions = {},
): Promise<Foxhound> {
  return open(await openOver(foxhound, transport, options));
}
