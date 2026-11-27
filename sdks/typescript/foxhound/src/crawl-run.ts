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
import {
  collieOperations,
  type Caller,
  type CollieOperationTypes,
  type RunFinishParams,
  type RunStarted,
  type RunStopParams,
} from "@goodfetch/collie-client/generated";
import {
  alsoInNamespace,
  attemptStoreOf,
  collieWire,
  ensureRunOpen,
  idOfLocator,
  pageOf,
  resumable,
  FOLLOWED_VALUE,
  paged,
  Sequence,
  type SequenceOptions,
  type Stream,
  type ClientConnection,
  type CollieWireType,
  fromWire,
  type RunHost,
  thrownException,
  warnEntry,
  type WireField,
  type WirePath,
} from "@goodfetch/collie-client/runtime";
import {
  checkCrawlStartable,
  checkPayload,
  checkRecords,
  checkSeedPayloads,
  declaredLabel,
  enqueueLinksOptions,
  handlerOf,
  isLinkPayload,
  payloadAndOptions,
  payloadOf,
  readRecord,
  recordsRefused,
  requestOptions,
  type Crawl,
  type CrawlContext,
  type Dataset,
  type DatasetRecord,
  type Label,
  type PayloadThen,
  type RequestOptions,
} from "./crawl.ts";
import { makeExport, type Export, type ExportOptions } from "./export.ts";
import { openSeedStream } from "./seed-stream.ts";
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
  type CrawlRequestOutcome,
  type CrawlRequestOutcomes,
  type CrawlRunProgress,
  type CrawlRunReport,
  type CrawlSettings,
  type CrawlStarted,
  type DatasetRecord as WireDatasetRecord,
  type Enqueued,
  type FoxhoundOperationTypes,
  type OutcomeKind,
  type OutcomesAfter,
  type RejectedRecord,
  type RejectedRecords,
  type RunExportParams,
  type RunOutcomesParams,
  type RunRetryFailedParams,
  type Seed,
  type SeedAttempt,
} from "./generated/index.ts";
import { settingsOnWire } from "./generated/settings.ts";
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

/**
 * How many rejected Records `rejectedRecords()` may have read before the
 * program has read any: no further page is asked once they are used, though
 * one page, of up to the engine's page size, may hand more.
 */
const REJECTED_CREDITS = 64;
/** How many rejected Records `rejectedRecords()` reads before it grants as many credits again. */
const REJECTED_REGRANT = 32;

/** What `records()` takes. */
export interface RecordsOptions {
  /** The number of the last Record a reader handled, read past; absent, from the first. */
  readonly after?: number | undefined;
  /** Stops reading: the stream is cancelled and the iteration fails with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** What `rejectedRecords()` takes. */
export interface RejectedRecordsOptions {
  /** The number of the last entry a reader handled, read past; absent, from the first. */
  readonly after?: number | undefined;
  /** Stops reading: the reads are cancelled and the iteration fails with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** What `outcomes()` takes. */
export interface OutcomesOptions {
  /** Only outcomes of these kinds; absent, every kind. */
  readonly kinds?: readonly OutcomeKind[] | undefined;
  /** Only outcomes of requests of these Labels (names or Label handles); absent, every Label. */
  readonly labels?: readonly (string | Label<object | undefined>)[] | undefined;
  /** Only outcomes of requests one of whose failed attempts failed with one of these Error codes. */
  readonly codes?: readonly string[] | undefined;
  /** Where to start: after this commit position, or `"now"` for the next commit on; absent, from the first. */
  readonly after?: OutcomesAfter | undefined;
  /** Stops reading: the reads are cancelled and the iteration fails with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** What `failedRequests()` takes. */
export interface FailedRequestsOptions {
  /** Also lists requests whose latest outcome is `blocked`; `false` by default. */
  readonly blocked?: boolean | undefined;
  /** Cancels the call with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/** What `retryFailed()` takes. */
export interface RetryFailedOptions {
  /** Also re-admits requests whose latest outcome is `blocked`; `false` by default. */
  readonly blocked?: boolean | undefined;
  /** Settings from the retry on, read when the retry returns an ended Run to `running`. */
  readonly settings?: CrawlSettings | undefined;
  /** Cancels the call with the signal's reason. */
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

  /** Its crawl, whose handlers its attempts run; none for a code-free crawl. */
  readonly #crawl: Crawl | undefined;
  readonly #crawlHost: CrawlHost;
  /** Each seed's original answer from `crawl.start`, read with no round trip. */
  readonly #seeds: readonly Enqueued[];
  /** The attempts running, each stopped when the Run is closed. */
  readonly #attempts = new Set<AbortController>();
  /** Whether `close()` was called: it stops the attempts and forgets the Run once. */
  #closed = false;

  /**
   * @internal `Foxhound.startCrawl` opens a crawl's Run; a program never
   * constructs one. A `detached` start's Run outlives its program.
   */
  constructor(
    host: RunHost,
    started: CrawlStarted | RunStarted,
    crawl: Crawl | undefined,
    crawlHost: CrawlHost,
    detached = false,
  ) {
    // Its report and progress are a crawl's (ADR 0120).
    super(crawlRunHost(host), started, detached);
    // A crawl's Run takes the `crawl` operations beside a Run's (ADR 0099).
    alsoInNamespace(this, "crawl");
    this.#crawl = crawl;
    this.#crawlHost = crawlHost;
    this.#seeds = "seeds" in started && started.seeds !== undefined ? [...started.seeds] : [];
  }

  /**
   * Lists each seed's answer, in the order the crawl's start gave them, as the
   * Frontier answered it when the Run began: added, with its crawl request's ID
   * and its URL as parsed, or not added, with its reason (`duplicate`,
   * `domain_policy`, `unsupported_scheme`, `invalid_url` or a cap's name). A
   * relative seed is `invalid_url`, since nothing gives it a base. So a crawl
   * that does nothing says why. Read from the Run's committed state, so it
   * works on a Run no program holds.
   *
   * Answered from `crawl.start`'s reply with no round trip, including after a
   * resume (ADR 0102).
   *
   * @experimental
   */
  seeds(): readonly Enqueued[] {
    return [...this.#seeds];
  }

  /**
   * Adds one seed to a running crawl, `run.addSeed`, admitted and answered as
   * soon as the Run state accepts it (written with the next Run-state batch at
   * the default durability, or committed before answering at `full`), and
   * answers as an enqueue does: added, with its crawl request's ID and its URL
   * as parsed, or not added, with its reason (`duplicate`,
   * `unsupported_scheme`, `invalid_url` or a cap's name). A relative seed is
   * `invalid_url`. An idle `crawl.keep_alive` crawl takes it and goes on.
   * Through a handler's context, naming its `attempt`, it is staged with that
   * attempt: once the attempt ended it fails with `attempt_ended`. A Run this
   * engine does not run as a live crawl, or a crawl already ending its Run,
   * fails with `run_not_found`.
   *
   * @experimental
   */
  addSeed(seed: Seed, options?: CallOptions): Promise<Enqueued>;
  addSeed<P extends object | undefined>(
    url: string,
    label: Label<P>,
    ...rest: PayloadThen<NoInfer<P>, RequestOptions & CallOptions>
  ): Promise<Enqueued>;
  async addSeed(
    seedOrUrl: Seed | string,
    labelOrOptions?: Label<object | undefined> | CallOptions,
    ...rest: unknown[]
  ): Promise<Enqueued> {
    return this.#addSeed(seedOrUrl, labelOrOptions, rest, undefined);
  }

  /**
   * Takes a stream of seeds for a running crawl, `run.addSeeds`, and answers
   * each as `run.add_seed` does, in order, so a million seeds never sit in
   * memory. The engine grants the call 256 credits with `$/credit`, one seed
   * each; the Client sends seeds in `$/seeds {id, seeds, end?}` batches, never
   * more than its credits, and the engine admits each batch (staged with
   * `attempt` when named, or written with the next Run-state batch at default
   * durability), sends its answers in one `$/enqueued {id, answers}` and
   * grants as many credits again. Once it answered the batch marked `end`, the
   * call answers how many seeds were added and how many were not. Cancelling
   * it keeps the seeds already answered. A batch beyond the credits granted,
   * or a seed that is no seed, fails the call with `invalid_params`; a Run
   * this engine does not run as a live crawl, or a crawl already ending its
   * Run, fails with `run_not_found`.
   *
   * @experimental
   */
  addSeeds(
    source: Iterable<Seed> | AsyncIterable<Seed>,
    options: CallOptions = {},
  ): Sequence<Enqueued> {
    return this.#addSeeds(source, options, undefined);
  }

  async #addSeed(
    seedOrUrl: Seed | string,
    labelOrOptions: Label<object | undefined> | CallOptions | undefined,
    rest: readonly unknown[],
    attempt: SeedAttempt | undefined,
    attemptSignal?: AbortSignal,
  ): Promise<Enqueued> {
    warnEntry(foxhoundOperations, "run.add_seed");
    attemptSignal?.throwIfAborted();
    ensureRunOpen(this, "run.add_seed");
    let wireSeed: Seed;
    let callOptions: CallOptions | undefined;
    if (typeof seedOrUrl === "string") {
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- overload `(url, label, ...rest)` binds the second argument to a `Label`.
      const label = labelOrOptions as Label<object | undefined>;
      const name = this.#crawl === undefined ? label.name : declaredLabel(this.#crawl, label);
      const given = payloadAndOptions<object | undefined, RequestOptions & CallOptions>(
        label,
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `PayloadThen` types what follows `label` as its Payload and options.
        rest as PayloadThen<object | undefined, RequestOptions & CallOptions>,
      );
      const payload = payloadOf(label, given.payload);
      await checkPayload(label, payload.payload);
      wireSeed = {
        url: seedOrUrl,
        label: name,
        ...payload,
        ...requestOptions(given.options ?? {}),
      };
      callOptions = given.options;
    } else {
      wireSeed = seedOrUrl;
      if (this.#crawl !== undefined) {
        checkCrawlStartable(this.#crawl, [wireSeed]);
        await checkSeedPayloads(this.#crawl, [wireSeed]);
      }
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- overload `(seed, options?)` binds the second argument to `CallOptions`.
      callOptions = labelOrOptions as CallOptions | undefined;
    }
    const signal = mergeSignals(attemptSignal, callOptions?.signal);
    const mergedOptions: CallOptions = signal === undefined ? {} : { signal };
    return this.#crawlHost.calls.call(
      "run.add_seed",
      {
        runId: this.id(),
        seed: wireSeed,
        ...(attempt === undefined ? {} : { attempt }),
      },
      mergedOptions,
    );
  }

  #addSeeds(
    source: Iterable<Seed> | AsyncIterable<Seed>,
    options: CallOptions,
    attempt: SeedAttempt | undefined,
  ): Sequence<Enqueued> {
    warnEntry(foxhoundOperations, "run.add_seeds");
    ensureRunOpen(this, "run.add_seeds");
    return openSeedStream({
      connection: this.#crawlHost.connection,
      runId: this.id(),
      crawl: this.#crawl,
      attempt,
      source,
      signal: options.signal,
    });
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
   * and the iteration ends once the engine answers. Each Record's value is
   * the Dataset's own type, parsed by its typed schema with each top-level
   * field the schema does not know kept; a Record the schema refuses ends
   * the iteration with `record_invalid` naming its `number`, past which
   * `after` resumes. Its stream is sent
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
  ): Sequence<DatasetRecord<O>> {
    return this.#records(dataset, options, "follows");
  }

  #records<I extends object, O>(
    dataset: Dataset<I, O>,
    options: RecordsOptions,
    reach: Reach,
  ): Sequence<DatasetRecord<O>> {
    warnEntry(foxhoundOperations, "run.records");
    ensureRunOpen(this, "run.records");
    const connection = this.#crawlHost.connection;
    const calls = this.#crawlHost.calls;
    const runId = this.id();
    const params: { run_id: string; dataset: string; after?: number; through?: number } = {
      run_id: runId,
      dataset: dataset.name,
      ...(options.after === undefined ? {} : { after: options.after }),
    };
    const send = (opened: Opened, sending: typeof params): Stream =>
      resumable<number>((sent) => connection.stream("run.records", sending, sent), opened, {
        read: (method, notification) => {
          const number = method === "$/record" ? recordNumberOf(notification) : undefined;
          return number === undefined ? undefined : { count: 1, last: number };
        },
        params: (last) => (last === undefined ? sending : { ...sending, after: last }),
      });
    return new Sequence({
      open: (opened) =>
        reach === "follows"
          ? send(opened, params)
          : // The last Record committed now, which `dataset.page` answers
            // as its `through` even for a page of one Record.
            sentAfter(
              async () => {
                const page = await calls.call(
                  "dataset.page",
                  { runId, dataset: dataset.name, columns: [], limit: 1 },
                  { signal: opened.signal },
                );
                return page.through;
              },
              (through) => send(opened, { ...params, through }),
            ),
      itemOf: (method, notification) => {
        const record = recordOf(method, notification);
        return record === undefined ? undefined : readRecord(dataset, record);
      },
      credits: RECORD_CREDITS,
      regrant: RECORD_REGRANT,
      signal: options.signal,
    });
  }

  /**
   * Reads one Dataset's rejection table from a crawl's committed state:
   * each Record rejected rather than committed, numbered from 1 in commit
   * order, with the Record as JSON, the reason, each issue's path, the Hook
   * when one rejected it, and the request's ID and URL. It starts at the
   * first entry, or after `after`, the number a reader last handled. While
   * the Run is live and nothing follows, it waits for the next commit; a
   * page says `ended` once the Run has ended, or has no live writer, and no
   * entry follows. With `through`, such as the `rejected` count
   * `dataset.list` answers, it reads no entry numbered past it and never
   * waits, a live Run's included: a page says `ended` once it reaches that
   * number or has read every committed entry. A reader never slows the Run. A Dataset the crawl does not
   * declare fails with `dataset_unknown`.
   *
   * Reads `dataset`'s rejected Records (ADRs 0103, 0104) page after page,
   * each Record's JSON as it was pushed, never parsed, and the iteration
   * ends once a page says the table is read to its end. Its first page is
   * asked when it is called (ADR 0081), each next one once the program has
   * read on, and `break`, `return()`, `close()`, `using` or releasing it
   * unread cancel the read at once. On a closed Run it throws `closed`,
   * sending nothing.
   *
   * @experimental
   */
  rejectedRecords<I extends object, O>(
    dataset: Dataset<I, O>,
    options: RejectedRecordsOptions = {},
  ): Sequence<RejectedRecord> {
    return this.#rejectedRecords(dataset, options, "follows");
  }

  /**
   * Writes an Export of one Dataset of a crawl's Run: the Records it committed
   * when the call is made, in number order, in the format named, as its
   * `format` value says: a table (`csv`, `xlsx`, `html_table`, `markdown`, and
   * typed, `parquet`, `avro` and `arrow_ipc`) has one column per field seen in
   * any of its Records, in first-seen order, and one row per Record. Every
   * text Export is UTF-8 with no BOM unless `export.csv.bom` puts one on CSV,
   * characters written as themselves (a character XML 1.0 cannot hold as
   * U+FFFD in XML and feeds) and each value as it was committed: a table cell
   * holds a string as its text, a number, `true` or `false` as JSON writes it,
   * nothing for `null` or a field its Record lacks, and an object or array as
   * its JSON text, or, in CSV under `export.csv.nested: flatten`, an object's
   * fields in dotted columns. A Record whose page was read past a Wall carries
   * its Wall marker, `{kind, vendor?}` as its Provenance holds it, as a
   * `#wall` field in every format but the feeds, where it is a category of the
   * scheme `urn:goodfetch:wall` naming its kind; `wall` set to `true` writes
   * only those Records, `false` only the others. `compression` `gzip` or
   * `zstd` compresses the whole file. Its `settings` and `project` are the run
   * layer and the project configuration its `export.*` settings resolve under;
   * a value the format cannot take fails with `setting_invalid`. Answers the
   * Export, with its Record count, its last Record's number, its size, its
   * SHA-256 and the run epoch of the Run's state it was written from, only
   * once its file is whole in the Run's folder, where `export.save_as` and
   * `export.stream` read it. Records committed after the call starts are not
   * in it. A write the engine's storage refuses fails with
   * `file_write_failed`, side `engine`, leaving no unfinished file, as does a
   * call while that storage is at the `critical` level, before any byte is
   * written. A Dataset the crawl does not declare fails with
   * `dataset_unknown`; an `xlsx` sheet past Excel's limits, with
   * `sheet_capacity_exceeded`.
   *
   * @experimental
   */
  async export<I extends object, O>(
    dataset: Dataset<I, O>,
    options: ExportOptions,
  ): Promise<Export> {
    warnEntry(foxhoundOperations, "run.export");
    ensureRunOpen(this, "run.export");
    const settings = settingsOnWire(options.settings);
    const project = this.#crawlHost.connection.project();
    const params: RunExportParams = {
      runId: this.id(),
      dataset: dataset.name,
      format: options.format,
      ...(options.compression === undefined ? {} : { compression: options.compression }),
      ...(options.wall === undefined ? {} : { wall: options.wall }),
      ...(settings === undefined ? {} : { settings }),
      ...(project === undefined ? {} : { project }),
    };
    const descriptor = await this.#crawlHost.calls.call("run.export", params, {
      signal: options.signal,
    });
    return makeExport(this.#crawlHost.connection, descriptor);
  }

  /**
   * A log of endings, not current state: a re-admitted request's new outcome
   * supersedes the old by `request.id` (ADRs 0102, 0117).
   *
   * Reads a crawl's outcomes from its committed state, in commit order: from
   * the first, after a position, or with `after: "now"` from the next commit.
   * Its filters, `kinds`, `labels` and `codes` (an Error code any of the
   * request's failed attempts failed with), run in the engine. While the Run
   * is live and nothing new matches, it waits for the next commit; a page says
   * `ended` once the Run has ended, or has no live writer, and nothing more
   * matches. A reader never slows the Run. A request `run.retry_failed`
   * re-admits ends again, and its later outcome supersedes the earlier one by
   * `request.id`.
   *
   * @experimental
   */
  outcomes(options: OutcomesOptions = {}): Sequence<CrawlRequestOutcome> {
    warnEntry(foxhoundOperations, "run.outcomes");
    ensureRunOpen(this, "run.outcomes");
    const calls = this.#crawlHost.calls;
    const runId = this.id();
    const kinds = options.kinds === undefined ? undefined : [...options.kinds];
    const labels =
      options.labels === undefined
        ? undefined
        : options.labels.map((label) => (typeof label === "string" ? label : label.name));
    const codes = options.codes === undefined ? undefined : [...options.codes];
    let take: (() => CrawlRequestOutcome | undefined) | undefined;
    return new Sequence({
      open: (opened) => {
        const reading = paged<CrawlRequestOutcomes, CrawlRequestOutcome>(
          {
            read: async (read, signal) => {
              ensureRunOpen(this, "run.outcomes");
              const after: OutcomesAfter | undefined = read ?? options.after;
              const params: RunOutcomesParams = {
                runId,
                ...(kinds === undefined ? {} : { kinds }),
                ...(labels === undefined ? {} : { labels }),
                ...(codes === undefined ? {} : { codes }),
                ...(after === undefined ? {} : { after }),
              };
              return calls.call("run.outcomes", params, { signal });
            },
            position: (page) => page.next,
            last: (page) => page.ended,
            items: (page) => page.outcomes,
          },
          opened,
        );
        take = reading.take;
        return reading.stream;
      },
      itemOf: (method) => (method === FOLLOWED_VALUE ? take?.() : undefined),
      credits: REJECTED_CREDITS,
      regrant: REJECTED_REGRANT,
      signal: options.signal,
    });
  }

  /**
   * Reads from a crawl's committed state the requests whose latest outcome is
   * `failed`, and with `blocked` also `blocked`, each with that outcome:
   * exactly those `run.retry_failed` re-admits with the same option. A skip is
   * a decision and is never listed.
   *
   * @experimental
   */
  async failedRequests(
    options: FailedRequestsOptions = {},
  ): Promise<readonly CrawlRequestOutcome[]> {
    warnEntry(foxhoundOperations, "run.failed_requests");
    ensureRunOpen(this, "run.failed_requests");
    const result = await this.#crawlHost.calls.call(
      "run.failed_requests",
      { runId: this.id(), blocked: options.blocked === true },
      { signal: options.signal },
    );
    return result.requests;
  }

  /**
   * Re-admits, in the same Run and its Datasets, every request of a crawl
   * whose latest outcome is `failed`, and with `blocked` also `blocked`:
   * exactly those `run.failed_requests` lists with the same option. Each keeps
   * its ID, Label, Payload and depth; its attempts count on, with a fresh
   * `crawl.retry.max`. A skip is a decision and is never re-admitted. It
   * answers how many it re-admitted, 0 when there was nothing to retry. On a
   * crawl this engine runs, the requests join its work at once, unless the
   * crawl is already ending its Run: the call then waits for that end and
   * re-admits them as on an ended Run. On an ended Run with requests to retry,
   * the Run returns to `running` under `settings`, as a resume does, its
   * Verdict and Completeness given anew when it ends, and the calling
   * Connection serves its handlers. A Run with no writer that has not ended
   * fails with `run_interrupted`: resume it first. A request to retry whose
   * Label the crawl no longer declares, which a resume may drop, fails the
   * call with `label_unknown`, naming that Label and those declared, and
   * nothing is re-admitted: on an ended Run, against the Labels its latest
   * start declared, before the Run returns to `running`. It is not sent again
   * after a lost Connection, which fails it with `connection_lost`:
   * `run.failed_requests` then lists the requests still to retry.
   *
   * @experimental
   */
  async retryFailed(options: RetryFailedOptions = {}): Promise<number> {
    warnEntry(foxhoundOperations, "run.retry_failed");
    ensureRunOpen(this, "run.retry_failed");
    const settings = settingsOnWire(options.settings);
    const project = this.#crawlHost.connection.project();
    const params: RunRetryFailedParams = {
      runId: this.id(),
      blocked: options.blocked === true,
      ...(settings === undefined ? {} : { settings }),
      ...(project === undefined ? {} : { project }),
    };
    const result = await this.#crawlHost.calls.call("run.retry_failed", params, {
      signal: options.signal,
    });
    return result.readmitted;
  }

  #rejectedRecords<I extends object, O>(
    dataset: Dataset<I, O>,
    options: RejectedRecordsOptions,
    reach: Reach,
  ): Sequence<RejectedRecord> {
    warnEntry(foxhoundOperations, "run.rejected_records");
    ensureRunOpen(this, "run.rejected_records");
    const calls = this.#crawlHost.calls;
    const runId = this.id();
    let take: (() => RejectedRecord | undefined) | undefined;
    return new Sequence({
      open: (opened) => {
        // The entries rejected now, numbered without gaps, so the count
        // `dataset.list` answers is the last one's number.
        let bound: Promise<number> | undefined;
        const through = async (signal: AbortSignal | undefined): Promise<number | undefined> => {
          if (reach === "follows") return undefined;
          bound ??= calls
            .call("dataset.list", { runId }, { signal })
            .then(
              (list) =>
                list.datasets.find((summary) => summary.name === dataset.name)?.rejected ?? 0,
            );
          return bound;
        };
        const reading = paged<RejectedRecords, RejectedRecord>(
          {
            read: async (read, signal) => {
              ensureRunOpen(this, "run.rejected_records");
              const after = read ?? options.after;
              const last = await through(signal);
              const params = {
                runId,
                dataset: dataset.name,
                ...(after === undefined ? {} : { after }),
                ...(last === undefined ? {} : { through: last }),
              };
              return calls.call("run.rejected_records", params, { signal });
            },
            position: (page) => page.next,
            last: (page) => page.ended,
            items: (page) => page.records,
          },
          opened,
        );
        take = reading.take;
        return reading.stream;
      },
      itemOf: (method) => (method === FOLLOWED_VALUE ? take?.() : undefined),
      credits: REJECTED_CREDITS,
      regrant: REJECTED_REGRANT,
      signal: options.signal,
    });
  }

  /**
   * Lets a Run finish and answers its Run report: a Run its program drives ends
   * at once, `finished` and `complete`; a crawl ends once its work items are
   * done, and a keep-alive crawl (`crawl.keep_alive`), which waits for new work
   * while nothing is left, ends once this call lets it and its work items are
   * done. A Run something stopped first answers its report saying `stopped`. A
   * Run that ended `failed` fails the call with the error that failed it, and a
   * write at its end that failed, such as its Plan file's, with that write's
   * error. Cancelling the call, or its `timeout` passing, which fails it with
   * `deadline_exceeded`, ends only the wait.
   *
   * The report is a crawl's (ADR 0120): a Run's report with its request
   * totals, its Records per Dataset, its Sinks, its sitemaps and the rules it
   * learned. Its Completeness is `incomplete` or `unverified` when a setting
   * cut the crawl short, its reason naming the setting and the count it
   * refused.
   *
   * @experimental
   */
  override finish(
    options: {
      readonly timeout?: RunFinishParams["timeout"] | undefined;
      readonly signal?: AbortSignal | undefined;
    } = {},
  ): Promise<CrawlRunReport> {
    // A crawl's Run reads `run.finish`'s answer as a crawl's report (`crawlRunHost`).
    return super.finish(options);
  }

  /**
   * Stops a Run at its next safe point with a Checkpoint, `stopped` and
   * `incomplete` as an abort by its program, resumable by its Run ID, and
   * answers its Run report; an ended Run's report is answered as it is. The
   * wait is bounded by `timeout.close`, or the call's `timeout`: past it, the
   * call fails with `deadline_exceeded` and the Run still stops at its next
   * safe point. A Run that ended `failed` fails the call as `run.finish` does.
   *
   * The report is a crawl's: no attempt in flight commits, and its work items
   * return to pending.
   *
   * @experimental
   */
  override stop(
    options: {
      readonly timeout?: RunStopParams["timeout"] | undefined;
      readonly signal?: AbortSignal | undefined;
    } = {},
  ): Promise<CrawlRunReport> {
    // A crawl's Run reads `run.stop`'s answer as a crawl's report (`crawlRunHost`).
    return super.stop(options);
  }

  /**
   * Reads a Run's report: at once for a Run that ended, and at its end for a
   * live one, which the read never ends. A Run another engine works is read
   * through that engine, and one no engine works from its committed state,
   * never taking its writer lock. A Run that has no live writer and has not
   * ended fails with `run_interrupted`, whose `fix` names a start with its Run
   * ID, and a Run ID the Run index does not list with `run_not_found`.
   *
   * The report is a crawl's.
   *
   * @experimental
   */
  override report(
    options: { readonly signal?: AbortSignal | undefined } = {},
  ): Promise<CrawlRunReport> {
    // A crawl's Run reads `run.report`'s answer as a crawl's report (`crawlRunHost`).
    return super.report(options);
  }

  /**
   * Reads a Run's progress: what it does and why it waits. Without `after`, it
   * answers at once; given `after`, the position of the value the reader last
   * read, it waits for a change and answers the latest value, so values in
   * between are coalesced. The value whose `last` is true ends the sequence:
   * the Run ended, and its `lifecycle` and `stop_reason` say how, or no engine
   * works it (`writer: none`) until a start with its Run ID resumes it. A Run
   * another engine works is read through that engine, the owner the Run index
   * names, and one no engine works from its committed state; a read never takes
   * the Run's writer lock. A Run ID the Run index does not list fails with
   * `run_not_found`.
   *
   * Each value is a crawl's (ADR 0120): a Run's progress with its requests by
   * state, its concurrency, the requests in flight, the domains it waits on and
   * whether it waits idle, read with `for await` as a Run's progress is.
   *
   * @experimental
   */
  override progress(
    options: { readonly signal?: AbortSignal | undefined } = {},
  ): Sequence<CrawlRunProgress> {
    // A crawl's Run reads `run.progress`'s answers as a crawl's progress (`crawlRunHost`).
    return super.progress(options);
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
    const crawl = this.#crawl;
    const handler = crawl === undefined ? undefined : handlerOf(crawl, label);
    if (crawl === undefined || handler === undefined) {
      throw new HandlerFailedError(`the Crawl attaches no handler to the Label ${label}`, {
        fix: "Attach one with `crawl.handle(label, handler)` before starting the crawl.",
        details: { label },
      });
    }
    const attempt = new AbortController();
    this.#attempts.add(attempt);
    try {
      await handler(this.#contextOf(crawl, request, AbortSignal.any([attempt.signal, served])));
      return {};
    } catch (error) {
      if (error instanceof GoodfetchError) throw error;
      throw handlerFailed(label, error);
    } finally {
      this.#attempts.delete(attempt);
    }
  }

  #contextOf(crawl: Crawl, request: CrawlRequestHandleParams, signal: AbortSignal): CrawlContext {
    const lease = { crawlRequestId: request.crawlRequest.id, leaseEpoch: request.leaseEpoch };
    const attempt = { runId: this.id(), ...lease };
    const calls = this.#crawlHost.calls;
    const options: CallOptions = { signal };
    const pageId = request.pageId ?? undefined;
    const page =
      pageId === undefined ? undefined : pageOf(this, pageId, request.session.id, signal);
    return {
      request: request.crawlRequest,
      run: {
        id: () => attempt.runId,
        keyValueStore: attemptStoreOf(this, lease, signal),
        records: (dataset, reading = {}) => this.#records(dataset, reading, "committed"),
        rejectedRecords: (dataset, reading = {}) =>
          this.#rejectedRecords(dataset, reading, "committed"),
        addSeed: (
          seedOrUrl: Seed | string,
          labelOrOptions?: Label<object | undefined> | CallOptions,
          ...rest: unknown[]
        ) => this.#addSeed(seedOrUrl, labelOrOptions, rest, lease, signal),
        addSeeds: (source, seedOptions = {}) =>
          this.#addSeeds(
            source,
            {
              ...seedOptions,
              signal:
                seedOptions.signal === undefined
                  ? signal
                  : AbortSignal.any([signal, seedOptions.signal]),
            },
            lease,
          ),
      },
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

/** Combines two optional abort signals without allocating when at most one is set. */
function mergeSignals(
  first: AbortSignal | undefined,
  second: AbortSignal | undefined,
): AbortSignal | undefined {
  if (first === undefined) return second;
  if (second === undefined) return first;
  return AbortSignal.any([first, second]);
}

/** Answers the engine's `crawl_request.handle` request for `run`. */
export function handleRequest(
  run: CrawlRun,
  request: CrawlRequestHandleParams,
  signal: AbortSignal,
): Promise<CrawlRequestHandled> {
  return handleIn(run, request, signal);
}

/**
 * The Run operations whose answers a crawl's Run reads as Foxhound's own
 * types (ADR 0120), each with the fields only Foxhound's type has beside
 * the Collie type the Connection reads it as first.
 */
const CRAWL_ANSWERS: Readonly<Partial<Record<keyof CollieOperationTypes, WirePath>>> = {
  "run.finish": crawlOwnFields("CrawlRunReport", "RunReport"),
  "run.stop": crawlOwnFields("CrawlRunReport", "RunReport"),
  "run.report": crawlOwnFields("CrawlRunReport", "RunReport"),
  "run.progress": crawlOwnFields("CrawlRunProgress", "RunProgress"),
};

/** Both Products' wire types, which a crawl's fields reach. */
const CRAWL_TYPES = { ...collieWire.types, ...foxhoundWire.types };

/**
 * `host` with the answers of `CRAWL_ANSWERS` read as a crawl's: the
 * Connection reads a Run's report and progress as Collie's, which keeps
 * the fields only a crawl's carry as sent, so these are read again as
 * Foxhound's types.
 */
function crawlRunHost(host: RunHost): RunHost {
  const calls = host.calls;
  const crawlCalls: Caller<CollieOperationTypes> = {
    call: async (operation, params, options) => {
      const answer = await calls.call(operation, params, options);
      const own = CRAWL_ANSWERS[operation];
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the answer keeps the operation's Collie type, which TypeScript cannot see `fromWire` keep: engine answers are not validated beyond `hello`'s protocol version (STYLE.md), and a crawl's report and progress are Collie's with Foxhound's fields read beside them.
      return (own === undefined ? answer : fromWire(answer, own, CRAWL_TYPES)) as typeof answer;
    },
  };
  return { ...host, calls: crawlCalls };
}

/** The fields only Foxhound's `type` has beside Collie's `base`. */
function crawlOwnFields(type: FoxhoundWireType, base: CollieWireType): WirePath {
  const known = fieldsOf(collieWire.types[base]);
  const own = Object.entries(fieldsOf(foxhoundWire.types[type])).filter(
    ([key]) => !Object.hasOwn(known, key),
  );
  return { fields: Object.fromEntries(own) };
}

/** The fields `path` names, none when it names no object. */
function fieldsOf(path: WirePath): Readonly<Record<string, WireField>> {
  return typeof path === "object" && "fields" in path ? path.fields : {};
}

/**
 * How far a read of a Run's Records goes: `follows` follows a live Run to
 * its end, as the program's own Run reads; `committed` reads through what
 * has committed when it is sent, then ends, as a handler's `ctx.run` reads,
 * whose Run cannot end while the handler holds its request (ADR 0103).
 */
type Reach = "follows" | "committed";

/** What a sequence's stream is opened with. */
type Opened = Parameters<SequenceOptions<unknown>["open"]>[0];

/**
 * A stream that asks `bound` first, then sends the one `open` opens
 * through it, the credits granted meanwhile handed on. A failed `bound`
 * fails the stream.
 */
function sentAfter(bound: () => Promise<number>, open: (through: number) => Stream): Stream {
  let stream: Stream | undefined;
  let granted = 0;
  const result = bound().then((through) => {
    stream = open(through);
    if (granted > 0) stream.grant(granted);
    return stream.result;
  });
  return {
    result,
    grant: (credits) => {
      if (stream === undefined) granted += credits;
      else stream.grant(credits);
    },
  };
}

/** The number of the Record a `$/record` carries, read from its wire form alone. */
function recordNumberOf(params: unknown): number | undefined {
  if (typeof params !== "object" || params === null || !("record" in params)) return undefined;
  const { record } = params;
  if (typeof record !== "object" || record === null || !("number" in record)) return undefined;
  return typeof record.number === "number" ? record.number : undefined;
}

/** The Record a `$/record` notification carries; none for any other notification. */
function recordOf(method: string, params: unknown): WireDatasetRecord | undefined {
  if (method !== "$/record" || typeof params !== "object" || params === null) return undefined;
  if (!("record" in params)) return undefined;
  const record = fromWire(
    params.record,
    { type: "DatasetRecord" satisfies FoxhoundWireType },
    foxhoundWire.types,
  );
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine messages are not validated beyond `hello`'s protocol version (STYLE.md): a `$/record` carries a Dataset's Record, its declared fields in camelCase and its points in time read as `Date`s.
  return record as WireDatasetRecord;
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
