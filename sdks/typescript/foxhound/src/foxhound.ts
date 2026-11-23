// Foxhound's Connection type (ADRs 0078, 0090 and 0095): `Foxhound.connect()`
// finds Foxhound's executable, starts it as an Attached engine with exactly
// the program's environment map, and finishes `hello`, naming Foxhound,
// before it returns, so a Collie engine refuses it there with
// `not_in_product`. `startCrawl` starts a crawl from a `Crawl` value and
// serves the engine's `crawl_request.handle` requests to its handlers.
// Everything else is the client package's common Connection, whose members
// `Collie` carries under the same names.
import type {
  CatalogTarget,
  ConnectOptions,
  ConnectOverOptions,
  Connection,
  EngineInfo,
  HandlerlessStartOptions,
  JSONValue,
  Limits,
  Money,
  RawCallOptions,
  RawParams,
  Run,
  RunView,
  SettingsExplanation,
  StartOptions,
} from "@goodfetch/collie-client";
import { RunNotFoundError } from "@goodfetch/collie-client";
import {
  fromWire,
  holdInRealm,
  openEngine,
  openOver,
  refuseHandlersOnEdge,
  type ClientConnection,
  type Transport,
  warnEntry,
} from "@goodfetch/collie-client/runtime";
import { checkCrawlStartable, checkSeedPayloads, crawlDeclaration, type Crawl } from "./crawl.ts";
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
import { settingsOnWire, type CrawlSettings, type FoxhoundSettings } from "./generated/settings.ts";
import { foxhoundWire, type FoxhoundWireType } from "./generated/wire.ts";
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

/** Options of `connectOver()`, with Foxhound's settings and Limits. */
export interface FoxhoundConnectOverOptions extends ConnectOverOptions<FoxhoundSettings> {
  readonly limits?: FoxhoundLimits | undefined;
}

/** Options of `connect()`, with Foxhound's settings and Limits. */
export interface FoxhoundConnectOptions extends ConnectOptions<FoxhoundSettings> {
  readonly limits?: FoxhoundLimits | undefined;
}

/** What starting a Run takes, with Foxhound's Limits. */
export interface FoxhoundStartOptions extends StartOptions {
  readonly limits?: FoxhoundLimits | undefined;
}

/** What starting a handler-less Run takes, with Foxhound's Limits. */
export interface FoxhoundHandlerlessStartOptions extends HandlerlessStartOptions {
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
    const calls = connection.caller<FoxhoundOperationTypes>(foxhoundWire);
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
      this.#handle(
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the engine's requests are not validated beyond `hello`'s protocol version (STYLE.md): `crawl_request.handle` carries its params as the catalog types them, read in camelCase.
        fromWire(
          params,
          { type: "CrawlRequestHandleParams" satisfies FoxhoundWireType },
          foxhoundWire.types,
        ) as CrawlRequestHandleParams,
        signal,
      ),
    );
    holdInRealm(this);
  }

  /**
   * Starts Foxhound's engine as an Attached engine and completes `hello`
   * before returning, so a project file that cannot be read
   * (`setting_invalid`, or `format_version_unsupported` for its version), a
   * missing executable (`engine_not_found`), a
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
   * Leaving the scope or `close()` stops the Run with a Checkpoint.
   * `finish()` ends it.
   *
   * In a service, close the Run on `SIGINT` and `SIGTERM`: the SDK installs
   * no signal handler, and a signal the program does not handle ends it as a
   * crash would (ADRs 0078, 0093).
   *
   * Starts a Run its program drives, and answers once it is started. A Run
   * ID names the Run for good, and a start under one acts on what the Run
   * index holds (ADR 0102): no such Run begins it, `new`; an interrupted
   * Run, stopped, failed, or running with no live writer, resumes from its
   * committed state at the next run epoch, `resumed`, as `run.resume` does;
   * a finished Run is answered as it ended with no handler called, `ended`.
   * A Run live with a writer, in this engine or another, fails with
   * `run_in_use`; a Run started with other inputs or by another caller with
   * `run_conflict` listing each difference; a Run the Run index lists whose
   * state is missing or unreadable with `run_state_unavailable`, never
   * starting afresh. A stopped Run resumes under its Run ID.
   *
   * @experimental
   */
  start(options?: FoxhoundStartOptions): Promise<Run> {
    return this.#connection.start(options);
  }

  /**
   * Starts a Recipe's run, a Run with no handler in the program, and answers
   * once it is started. Its Run ID works as `run.start`'s. With `detached`,
   * the Run outlives the program that started it: closing its value frees
   * only the value, `run.finish` waits for its end and `run.stop` stops it.
   *
   * @experimental
   */
  startRecipe(recipe: string, options?: FoxhoundHandlerlessStartOptions): Promise<Run> {
    return this.#connection.startRecipe(recipe, options);
  }

  /**
   * Starts a code-free catalog run, a Run with no handler in the program
   * whose verbs come from the catalog (ADR 0036): an Extraction of a page by
   * a schema, `extract`, an Agent task, `agent`, or a Plan replayed with no
   * model call, `plan_replay`; and answers once it is started. Its Run ID
   * works as `run.start`'s. With `detached`, the Run outlives the program
   * that started it: closing its value frees only the value, `run.finish`
   * waits for its end and `run.stop` stops it.
   *
   * @experimental
   */
  startCatalogRun(target: CatalogTarget, options?: FoxhoundHandlerlessStartOptions): Promise<Run> {
    return this.#connection.startCatalogRun(target, options);
  }

  /**
   * Opens a view of the Run `runId` names, one another program started such
   * as a detached Run, with no round trip (ADR 0102). The view takes no
   * writer role, and closing it frees only the view. Fails with `closed`
   * once the Connection is closed.
   */
  openRun(runId: string): Promise<RunView> {
    return this.#connection.openRun(runId);
  }

  /**
   * Starts a crawl from its seeds, and answers once its Run is started. The
   * engine then leases each work item to the program's handler as a
   * `crawl_request.handle` request, and the crawl's Run finishes once every
   * work item is done, or, with `crawl.keep_alive`, once `run.finish` lets
   * it and every work item is done. A Run ID works as `run.start`'s.
   *
   * Each work item goes to the handler of its Label, on this process's
   * event loop (ADR 0093). Before anything is sent, a Label without a
   * handler fails with `label_unhandled`, and a seed whose Label's name
   * `crawl` does not declare with `label_unknown` (a seed carries its
   * Label's name only), and a seed whose Payload its Label's schema
   * refuses with `payload_invalid`, naming each issue's path; the engine
   * checks each Payload again, its size too (ADR 0082). On an edge
   * runtime, whose request ends before the handlers would run, it fails
   * first with `edge_handlers_unsupported` (ADR 0049): the crawl runs as a
   * Recipe on the Daemon instead.
   *
   * @experimental
   */
  async startCrawl(crawl: Crawl, options: StartCrawlOptions): Promise<CrawlRun> {
    warnEntry(foxhoundOperations, "crawl.start");
    refuseHandlersOnEdge({
      operation: "crawl.start",
      names: foxhound.operationNames,
      handlers: "the crawl's handlers",
    });
    checkCrawlStartable(crawl, options.seeds);
    await checkSeedPayloads(crawl, options.seeds);
    const settings = settingsOnWire(options.settings);
    // The start carries the project file `connect()` read (ADR 0083).
    const project = this.#connection.project();
    const params: CrawlStartParams = {
      ...crawlDeclaration(crawl),
      seeds: [...options.seeds],
      ...(options.runId === undefined ? {} : { runId: options.runId }),
      ...(options.tags === undefined ? {} : { tags: [...options.tags] }),
      ...(settings === undefined ? {} : { settings }),
      ...(project === undefined ? {} : { project }),
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
   * Shows each setting's effective value, the layer that set it and the
   * layer that locked it, from the built-in defaults, the Operator policy,
   * user configuration, the project configuration given and the run layer:
   * the Connection's own settings with the run values given over them, or a
   * live Run's as it opened when `run_id` names one; and lists the keys only
   * the other Product honours as that Product's.
   *
   * What the Connection runs with (ADR 0083), which each Run it starts
   * inherits: each setting under `keys`, each a key or a prefix of keys
   * such as `timeout`, every one when left out, with the origin that set
   * it too, as `goodfetch config explain` shows them, the project file
   * `connect()` read as the project configuration. Fails with `closed`
   * once the Connection is closed.
   */
  explainSettings(keys?: readonly string[]): Promise<SettingsExplanation> {
    return this.#connection.explainSettings(keys);
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
    if (!this.#crawlRuns.has(request.runId)) {
      await Promise.allSettled(this.#starting);
    }
    const run = this.#crawlRuns.get(request.runId);
    if (run === undefined) {
      throw new RunNotFoundError(
        `the crawl request ${request.crawlRequest.id} names the Run ${request.runId}, which no crawl started on this Connection runs`,
        { details: { run_id: request.runId } },
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
