// A crawl's Run (ADRs 0093, 0102 and 0103): the Run `Foxhound.startCrawl`
// opens, whose handlers the engine reaches with `crawl_request.handle`
// requests, one per attempt at a work item. Each attempt runs its Label's
// handler on the event loop with a `CrawlContext` bound to its lease, and
// is answered once the handler's result settles; closing the Run stops
// every attempt still running. `records(dataset)` reads a Dataset's
// committed Records as the engine streams them, one per credit granted.
import {
  ClosedError,
  GoodfetchError,
  Run,
  type CallOptions,
  type Page,
} from "@goodfetch/collie-client";
import type { Caller, RunStarted } from "@goodfetch/collie-client/generated";
import {
  ensureRunOpen,
  idOfLocator,
  pageOf,
  type ClientConnection,
  withTimes,
  type RunHost,
} from "@goodfetch/collie-client/runtime";
import {
  declaredLabel,
  enqueueLinksOptions,
  handlerOf,
  payloadOf,
  requestOptions,
  wireObject,
  type Crawl,
  type CrawlContext,
  type Dataset,
} from "./crawl.ts";
import { HandlerFailedError, ResponseNotDocumentError } from "./generated/errors.ts";
import type {
  CrawlRequestHandleParams,
  CrawlRequestHandled,
  DatasetRecord,
  FoxhoundOperationTypes,
} from "./generated/index.ts";
import { foxhoundTimes } from "./generated/times.ts";

/** How many Records `records()` lets the engine send before the program has read any. */
const RECORD_CREDITS = 64;
/** How many Records `records()` reads before it grants as many credits again. */
const RECORD_REGRANT = 32;

/** What a crawl's Run reaches its Connection through, beyond a Run's own. */
export interface CrawlHost {
  /** The Connection. */
  readonly connection: ClientConnection;
  /** Foxhound's calls on it. */
  readonly calls: Caller<FoxhoundOperationTypes>;
  /** Forgets a closed crawl's Run. */
  forget(run: CrawlRun): void;
}

/** What `records()` takes. */
export interface RecordsOptions {
  /** Stops reading: the stream is cancelled and the iteration fails with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

let handleIn: (
  run: CrawlRun,
  request: CrawlRequestHandleParams,
  signal: AbortSignal,
) => Promise<CrawlRequestHandled>;

/** A crawl's Run, opened by `Foxhound.startCrawl` (ADR 0102). */
export class CrawlRun extends Run {
  static {
    handleIn = (run, request, signal) => run.#handle(request, signal);
  }

  readonly #crawl: Crawl;
  readonly #crawlHost: CrawlHost;
  /** The attempts running, each stopped when the Run is closed. */
  readonly #attempts = new Set<AbortController>();
  /** Whether `close()` was called: it stops the attempts and forgets the Run once. */
  #closed = false;

  /** @internal `Foxhound.startCrawl` opens a crawl's Run; a program never constructs one. */
  constructor(host: RunHost, started: RunStarted, crawl: Crawl, crawlHost: CrawlHost) {
    super(host, started);
    this.#crawl = crawl;
    this.#crawlHost = crawlHost;
  }

  /**
   * Reads `dataset`'s committed Records in number order (ADR 0103), each
   * as the engine streams it: while the Run is live it follows the Run,
   * and once the Run has ended and every Record is read, the iteration
   * ends. Leaving the loop early cancels the stream. On a closed Run it
   * fails with `closed`.
   */
  async *records<I extends object>(
    dataset: Dataset<I>,
    options: RecordsOptions = {},
  ): AsyncGenerator<DatasetRecord, void, undefined> {
    ensureRunOpen(this, "run.records");
    const stop = new AbortController();
    const signal =
      options.signal === undefined ? stop.signal : AbortSignal.any([options.signal, stop.signal]);
    const received: DatasetRecord[] = [];
    let wake: (() => void) | undefined;
    let ended = false;
    const stream = this.#crawlHost.connection.stream(
      "run.records",
      { run_id: this.id(), dataset: dataset.name },
      {
        signal,
        notified: (method, params) => {
          const record = recordOf(method, params);
          if (record !== undefined) {
            received.push(record);
            wake?.();
          }
        },
      },
    );
    const settled = (): void => {
      ended = true;
      wake?.();
    };
    void stream.result.then(settled, settled);
    stream.grant(RECORD_CREDITS);
    let read = 0;
    try {
      for (;;) {
        // Records already received are not read once the program stops reading.
        options.signal?.throwIfAborted();
        const record = received.shift();
        if (record !== undefined) {
          yield record;
          read += 1;
          if (read >= RECORD_REGRANT && !ended) {
            stream.grant(read);
            read = 0;
          }
        } else if (ended) {
          // Answers the stream's end, or fails with its error.
          await stream.result;
          return;
        } else {
          await new Promise<void>((resolve) => {
            wake = resolve;
          });
          wake = undefined;
        }
      }
    } finally {
      if (!ended) {
        stop.abort(new ClosedError("the program stopped reading the Records", {}));
      }
    }
  }

  /**
   * Closes the Run; idempotent. Every attempt still running is stopped
   * first: its context's calls fail with `closed` and the handler's page
   * answers no more. A Run still running is then stopped as `close()`
   * stops any Run.
   */
  override close(): Promise<void> {
    if (this.#closed) return super.close();
    this.#closed = true;
    for (const attempt of this.#attempts) {
      attempt.abort(
        new ClosedError(`the Run ${this.id()} is closed, which stopped the handler's attempt`, {
          fix: "Keep the Run open until its crawl is done, or start it again by its Run ID.",
        }),
      );
    }
    this.#attempts.clear();
    this.#crawlHost.forget(this);
    return super.close();
  }

  async #handle(
    request: CrawlRequestHandleParams,
    served: AbortSignal,
  ): Promise<CrawlRequestHandled> {
    const { label } = request.crawl_request;
    const handler = handlerOf(this.#crawl, label);
    if (handler === undefined) {
      throw new HandlerFailedError(`the Crawl attaches no handler to the Label ${label}`, {
        fix: "Attach one with `crawl.handle(label, handler)` before starting the crawl.",
        details: { label },
      });
    }
    const attempt = new AbortController();
    this.#attempts.add(attempt);
    try {
      await handler(this.#contextOf(request, AbortSignal.any([attempt.signal, served])));
      return {};
    } catch (error) {
      if (error instanceof GoodfetchError) throw error;
      throw new HandlerFailedError(
        `the handler of the Label ${label} failed: ${messageOf(error)}`,
        {
          cause: error,
        },
      );
    } finally {
      this.#attempts.delete(attempt);
    }
  }

  #contextOf(request: CrawlRequestHandleParams, signal: AbortSignal): CrawlContext {
    const attempt = {
      run_id: this.id(),
      crawl_request_id: request.crawl_request.id,
      lease_epoch: request.lease_epoch,
    };
    const calls = this.#crawlHost.calls;
    const crawl = this.#crawl;
    const options: CallOptions = { signal };
    const pageId = request.page_id ?? undefined;
    const page = pageId === undefined ? undefined : pageOf(this, pageId, signal);
    return {
      get page(): Page {
        if (page === undefined) {
          throw new ResponseNotDocumentError(
            `the crawl request ${request.crawl_request.id}'s response is not a document, so its handler has no page`,
            {},
          );
        }
        return page;
      },
      async enqueue(url, label, payload, requestOptionsGiven = {}) {
        signal.throwIfAborted();
        return calls.call(
          "crawl_request.enqueue",
          {
            ...attempt,
            url,
            label: declaredLabel(crawl, label),
            ...payloadOf(payload),
            ...requestOptions(requestOptionsGiven),
          },
          options,
        );
      },
      async enqueueLinks(locator, label, payload, optionsGiven = {}) {
        signal.throwIfAborted();
        return calls.call(
          "crawl_request.enqueue_links",
          {
            ...attempt,
            locator: idOfLocator(locator),
            label: declaredLabel(crawl, label),
            ...payloadOf(payload),
            ...enqueueLinksOptions(optionsGiven),
          },
          options,
        );
      },
      async push(dataset, records) {
        signal.throwIfAborted();
        // A Record is a JSON object, never a list, so a list is the Records of one push.
        const [first, ...rest] = Array.isArray(records) ? records : [records];
        // An empty list pushes no Record, so it sends nothing.
        if (first === undefined) return;
        await calls.call(
          "crawl_request.push",
          {
            ...attempt,
            dataset: dataset.name,
            records: [wireObject(first), ...rest.map(wireObject)],
          },
          options,
        );
      },
    };
  }
}

/** Answers the engine's `crawl_request.handle` request for `run`. */
export function handleRequest(
  run: CrawlRun,
  request: CrawlRequestHandleParams,
  signal: AbortSignal,
): Promise<CrawlRequestHandled> {
  return handleIn(run, request, signal);
}

/** The Record a `$/record` notification carries; none for any other notification. */
function recordOf(method: string, params: unknown): DatasetRecord | undefined {
  if (method !== "$/record" || typeof params !== "object" || params === null) return undefined;
  if (!("record" in params)) return undefined;
  const record = withTimes(params.record, foxhoundTimes.types.DatasetRecord, foxhoundTimes.types);
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine messages are not validated beyond `hello`'s protocol version (STYLE.md): a `$/record` carries a Dataset's Record, its points in time read as `Date`s.
  return record as DatasetRecord;
}

/** The message of a thrown value, whatever was thrown. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
