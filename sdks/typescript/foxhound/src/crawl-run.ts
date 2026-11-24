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
import { collieOperations, type Caller, type RunStarted } from "@goodfetch/collie-client/generated";
import {
  alsoInNamespace,
  ensureRunOpen,
  idOfLocator,
  pageOf,
  resumable,
  Sequence,
  type ClientConnection,
  fromWire,
  type RunHost,
  thrownException,
  warnEntry,
} from "@goodfetch/collie-client/runtime";
import {
  checkPayload,
  checkRecords,
  declaredLabel,
  enqueueLinksOptions,
  handlerOf,
  isLinkPayload,
  payloadAndOptions,
  payloadOf,
  recordsRefused,
  requestOptions,
  type Crawl,
  type CrawlContext,
  type Dataset,
} from "./crawl.ts";
import {
  HandlerFailedError,
  RecordInvalidError,
  ResponseNotDocumentError,
} from "./generated/errors.ts";
import {
  foxhoundOperations,
  type CrawlRequestEnqueueLinksParams,
  type CrawlRequestHandleParams,
  type CrawlRequestHandled,
  type DatasetRecord,
  type FoxhoundOperationTypes,
} from "./generated/index.ts";
import { foxhoundWire, type FoxhoundWireType } from "./generated/wire.ts";

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
    // A crawl's Run takes the `crawl` operations beside a Run's (ADR 0099).
    alsoInNamespace(this, "crawl");
    this.#crawl = crawl;
    this.#crawlHost = crawlHost;
  }

  /**
   * Reads the committed Records of one Dataset of a crawl's Run in number
   * order, from the first, or after the number `after`, which resumes a
   * reader from the last Record it handled. Each Record goes in a
   * `$/record` notification with its number, its value, its request's ID
   * and URL and when it was committed, one per credit granted with
   * `$/credit`. While the Run is live it follows the Run, sending each
   * Record once it commits; once the Run has ended, or has no live writer,
   * and every Record is sent, it answers how many it sent and the last
   * one's number. With `through`, such as the number `dataset.page`
   * answers as the last committed, it reads no Record numbered past it and
   * answers once it sent every Record through it, a live Run's included.
   * Records are read from the Run's committed state on a connection of
   * their own, so a reader that grants no credit never slows the crawl.
   * With `wall` set to `true` it reads only the Records whose page was
   * read past a Wall, which their Provenance marks, and with `false` only
   * the others (ADR 0052). A Dataset the crawl does not declare fails with
   * `dataset_unknown`.
   *
   * Reads `dataset`'s Records (ADR 0103), each as the engine streams it,
   * and the iteration ends once the engine answers. Its stream is sent
   * when it is called (ADR 0081), and `break`,
   * `return()`, `close()`, `using` or releasing it unread cancel it at
   * once; after a remote Daemon's dropped link, it is sent again after
   * the last Record read (spec 606). On a closed Run it throws `closed`,
   * sending nothing.
   *
   * @experimental
   */
  records<I extends object, O>(
    dataset: Dataset<I, O>,
    options: RecordsOptions = {},
  ): Sequence<DatasetRecord> {
    warnEntry(foxhoundOperations, "run.records");
    ensureRunOpen(this, "run.records");
    const connection = this.#crawlHost.connection;
    const params = { run_id: this.id(), dataset: dataset.name };
    return new Sequence({
      open: (opened) =>
        resumable<number>((sent) => connection.stream("run.records", params, sent), opened, {
          read: (method, notification) => {
            const number = method === "$/record" ? recordNumberOf(notification) : undefined;
            return number === undefined ? undefined : { count: 1, last: number };
          },
          params: (last) => (last === undefined ? params : { ...params, after: last }),
        }),
      itemOf: recordOf,
      credits: RECORD_CREDITS,
      regrant: RECORD_REGRANT,
      signal: options.signal,
    });
  }

  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint.
   * `finish()` ends it.
   *
   * Closes the Run; idempotent. Every attempt still running is stopped
   * first: its context's calls fail with `closed` and the handler's page
   * answers no more. A Run still running is then stopped as `close()`
   * stops any Run.
   *
   * Stops a Run at its next safe point, `stopped` and `incomplete`: no
   * attempt in flight commits, and its work items return to pending.
   * Answers its Run report; an ended Run's report is answered as it is. A
   * Run whose engine died is stopped by this engine, which takes its writer
   * role at the next run epoch; a Run another engine writes is stopped by
   * that engine, the owner the Run index names.
   *
   * @experimental
   */
  override close(): Promise<void> {
    warnEntry(collieOperations, "run.abort");
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
    const { label } = request.crawlRequest;
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
      throw handlerFailed(label, error);
    } finally {
      this.#attempts.delete(attempt);
    }
  }

  #contextOf(request: CrawlRequestHandleParams, signal: AbortSignal): CrawlContext {
    const attempt = {
      runId: this.id(),
      crawlRequestId: request.crawlRequest.id,
      leaseEpoch: request.leaseEpoch,
    };
    const calls = this.#crawlHost.calls;
    const crawl = this.#crawl;
    const options: CallOptions = { signal };
    const pageId = request.pageId ?? undefined;
    const page =
      pageId === undefined ? undefined : pageOf(this, pageId, request.session.id, signal);
    return {
      get page(): Page {
        if (page === undefined) {
          throw new ResponseNotDocumentError(
            `the crawl request ${request.crawlRequest.id}'s response is not a document, so its handler has no page`,
            {},
          );
        }
        return page;
      },
      async enqueue(url, label, ...rest) {
        warnEntry(foxhoundOperations, "crawl_request.enqueue");
        signal.throwIfAborted();
        const name = declaredLabel(crawl, label);
        const given = payloadAndOptions(label, rest);
        const payload = payloadOf(label, given.payload);
        await checkPayload(label, payload.payload);
        // A relative URL resolves against the base URL of the page's document.
        const on = pageId === undefined ? {} : { pageId };
        return calls.call(
          "crawl_request.enqueue",
          {
            ...attempt,
            ...on,
            url,
            label: name,
            ...payload,
            ...requestOptions(given.options ?? {}),
          },
          options,
        );
      },
      async enqueueLinks(locator, label, ...rest) {
        warnEntry(foxhoundOperations, "crawl_request.enqueue_links");
        signal.throwIfAborted();
        const name = declaredLabel(crawl, label);
        const given = payloadAndOptions(label, rest);
        const on = { ...attempt, locator: idOfLocator(locator) };
        let payloads: Pick<CrawlRequestEnqueueLinksParams, "payload" | "payloads">;
        if (isLinkPayload(given.payload)) {
          // The function reads each matched element before any link is sent.
          const payloadOfLink = given.payload;
          const { links } = await calls.call("crawl_request.links", on, options);
          const answers: unknown[] = [];
          for (const link of links) {
            const answer = await payloadOfLink(link);
            if (answer === null) {
              answers.push(null);
              continue;
            }
            const { payload } = payloadOf(label, answer);
            await checkPayload(label, payload);
            answers.push(payload ?? null);
          }
          payloads = { payloads: answers };
        } else {
          payloads = payloadOf(label, given.payload);
          await checkPayload(label, payloads.payload);
        }
        return calls.call(
          "crawl_request.enqueue_links",
          { ...on, label: name, ...payloads, ...enqueueLinksOptions(given.options ?? {}) },
          options,
        );
      },
      async push(dataset, records) {
        warnEntry(foxhoundOperations, "crawl_request.push");
        signal.throwIfAborted();
        // A Record is a JSON object, never a list, so a list is the Records of one push.
        const pushed: readonly unknown[] = Array.isArray(records) ? records : [records];
        const { held, refused } = await checkRecords(dataset, pushed);
        const [first, ...rest] = held;
        // A push left with no Record to send, an empty list's, sends nothing.
        if (first === undefined) {
          if (refused.length > 0) throw recordsRefused(dataset.name, refused);
          return;
        }
        try {
          await calls.call(
            "crawl_request.push",
            {
              ...attempt,
              dataset: dataset.name,
              records: [first.record, ...rest.map(({ record }) => record)],
            },
            options,
          );
        } catch (error) {
          // The engine names a Record by its position among those sent.
          if (refused.length === 0 || !(error instanceof RecordInvalidError)) throw error;
          const sent = error.typedDetails?.records ?? [];
          const all = [
            ...refused,
            ...sent.map(({ position, issues }) => ({
              position: held[position]?.position ?? position,
              issues,
            })),
          ];
          throw recordsRefused(
            dataset.name,
            all.toSorted((a, b) => a.position - b.position),
          );
        }
        if (refused.length > 0) throw recordsRefused(dataset.name, refused);
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

/** The number of the Record a `$/record` carries, read from its wire form alone. */
function recordNumberOf(params: unknown): number | undefined {
  if (typeof params !== "object" || params === null || !("record" in params)) return undefined;
  const { record } = params;
  if (typeof record !== "object" || record === null || !("number" in record)) return undefined;
  return typeof record.number === "number" ? record.number : undefined;
}

/** The Record a `$/record` notification carries; none for any other notification. */
function recordOf(method: string, params: unknown): DatasetRecord | undefined {
  if (method !== "$/record" || typeof params !== "object" || params === null) return undefined;
  if (!("record" in params)) return undefined;
  const record = fromWire(
    params.record,
    { type: "DatasetRecord" satisfies FoxhoundWireType },
    foxhoundWire.types,
  );
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine messages are not validated beyond `hello`'s protocol version (STYLE.md): a `$/record` carries a Dataset's Record, its declared fields in camelCase and its points in time read as `Date`s.
  return record as DatasetRecord;
}

/**
 * The `handler_failed` of a handler that threw `thrown`, its own exception
 * (ADRs 0079 and 0109): the message always names the exception's type and
 * message, and the details carry them with the frames of its stack, the
 * stack's text past the line that repeats them.
 */
function handlerFailed(label: string, thrown: unknown): HandlerFailedError {
  const { type, message } = thrownException(thrown);
  const stack = thrown instanceof Error ? (thrown.stack ?? "") : "";
  // V8's header line: the type alone when the message is empty.
  const header = message === "" ? type : `${type}: ${message}`;
  const frames =
    stack === header
      ? ""
      : stack.startsWith(`${header}\n`)
        ? stack.slice(header.length + 1)
        : stack;
  return new HandlerFailedError(
    `the handler of the Label ${label} failed with ${type}: ${message}`,
    {
      cause: thrown,
      details: {
        exception_type: type,
        exception_message: message,
        exception_stack: frames,
      },
    },
  );
}
