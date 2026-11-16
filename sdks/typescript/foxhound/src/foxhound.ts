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
  Run,
  StartOptions,
} from "@goodfetch/collie-client";
import { RunNotFoundError } from "@goodfetch/collie-client";
import {
  holdInRealm,
  openAttached,
  openOver,
  type ClientConnection,
  type Transport,
} from "@goodfetch/collie-client/runtime";
import { crawlDeclaration, type Crawl } from "./crawl.ts";
import { CrawlRun, handleRequest, type CrawlHost } from "./crawl-run.ts";
import {
  FoxhoundOperations,
  type CrawlRequestHandleParams,
  type CrawlRequestHandled,
  type CrawlStartParams,
  type FoxhoundOperationTypes,
  type RequestSeed,
} from "./generated/index.ts";
import { foxhound } from "./product.ts";

/** What starting a crawl takes (ADR 0093). */
export interface StartCrawlOptions {
  /** The requests the crawl starts from: at least one. */
  readonly seeds: readonly [RequestSeed, ...RequestSeed[]];
  /** The Run's ID; the engine generates one when it is left out. An existing ID returns that Run. */
  readonly runId?: string | undefined;
  /** Its tags: at most 16, each at most 64 characters (ADR 0036). */
  readonly tags?: readonly string[] | undefined;
  /** Its settings, nested by key, such as `crawl.handler.concurrency`. */
  readonly settings?: CrawlStartParams["settings"] | undefined;
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
    const calls = connection.caller<FoxhoundOperationTypes>();
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
  static async connect(options: ConnectOptions = {}): Promise<Foxhound> {
    return open(await openAttached(foxhound, options));
  }

  /** The engine reached: its kind, Product, Product version and protocol version. Read without a round trip. */
  engine(): EngineInfo {
    return this.#connection.engine();
  }

  /**
   * Starts a Run and returns once it is started (ADR 0102); a `runId`
   * naming an existing Run returns that Run. `finish()` ends it; closing it
   * unfinished stops it, resumable by its Run ID.
   */
  start(options?: StartOptions): Promise<Run> {
    return this.#connection.start(options);
  }

  /**
   * Starts `crawl` from its seeds and returns once its Run is started (ADR
   * 0093). The engine then leases each work item to the handler of its
   * Label, on this process's event loop, and the Run finishes once every
   * work item is done. A `runId` naming an existing Run returns that Run.
   */
  async startCrawl(crawl: Crawl, options: StartCrawlOptions): Promise<CrawlRun> {
    const params: CrawlStartParams = {
      ...crawlDeclaration(crawl),
      seeds: [...options.seeds],
      ...(options.runId === undefined ? {} : { run_id: options.runId }),
      ...(options.tags === undefined ? {} : { tags: [...options.tags] }),
      ...(options.settings === undefined ? {} : { settings: options.settings }),
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
  options: ConnectOverOptions = {},
): Promise<Foxhound> {
  return open(await openOver(foxhound, transport, options));
}
