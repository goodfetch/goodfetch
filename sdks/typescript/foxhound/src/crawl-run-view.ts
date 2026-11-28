// A read-only view of a crawl's Run another program started (ADRs 0102 and
// 0103; issue 1489), opened by `Foxhound.openCrawlRun()` with no round trip
// and no writer lock. Reads its progress, report, trace, Key-value store,
// outcomes, failed requests, Records, rejected Records and Exports, typed by
// the program's own `Dataset` handles. Closing it frees only the view: the
// Run goes on.
import { RunView } from "@goodfetch/collie-client";
import { ensureRunViewOpen, type RunHost, type Sequence } from "@goodfetch/collie-client/runtime";
import type { Dataset, DatasetRecord } from "./crawl.ts";
import {
  crawlRunHost,
  exportCrawlDataset,
  readCrawlFailedRequests,
  readCrawlOutcomes,
  readCrawlRecords,
  readCrawlRejectedRecords,
  type CrawlHost,
  type FailedRequestsOptions,
  type OutcomesOptions,
  type RecordsOptions,
  type RejectedRecordsOptions,
} from "./crawl-run.ts";
import type { Export, ExportOptions } from "./export.ts";
import type {
  CrawlRequestOutcome,
  CrawlRunProgress,
  CrawlRunReport,
  RejectedRecord,
} from "./generated/index.ts";

/**
 * A read-only view of a crawl's Run named by its Run ID, opened by
 * `Foxhound.openCrawlRun()` with no round trip (ADRs 0102 and 0103). It takes
 * no writer lock and never resumes the Run; closing it frees only the view.
 *
 * On a Run with no live writer, its sequences end after what is committed and
 * `report()` raises `run_interrupted` whose `fix` names a start with the Run's
 * ID.
 */
export class CrawlRunView extends RunView {
  readonly #crawlHost: CrawlHost;

  /** @internal `Foxhound.openCrawlRun()` opens a view; a program never constructs one. */
  constructor(host: RunHost, id: string, crawlHost: CrawlHost) {
    super(crawlRunHost(host), id);
    this.#crawlHost = crawlHost;
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
   * Each value is a crawl's (ADR 0120).
   *
   * @experimental
   */
  override progress(
    options: { readonly signal?: AbortSignal | undefined } = {},
  ): Sequence<CrawlRunProgress> {
    return super.progress(options);
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
   * @experimental
   */
  records<I extends object, O>(
    dataset: Dataset<I, O>,
    options: RecordsOptions = {},
  ): Sequence<DatasetRecord<O>> {
    return readCrawlRecords(
      this.#crawlHost,
      this.id(),
      (operation) => {
        ensureRunViewOpen(this, operation);
      },
      dataset,
      options,
      "follows",
    );
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
   * number or has read every committed entry. A reader never slows the Run. A
   * Dataset the crawl does not declare fails with `dataset_unknown`.
   *
   * @experimental
   */
  rejectedRecords<I extends object, O>(
    dataset: Dataset<I, O>,
    options: RejectedRecordsOptions = {},
  ): Sequence<RejectedRecord> {
    return readCrawlRejectedRecords(
      this.#crawlHost,
      this.id(),
      (operation) => {
        ensureRunViewOpen(this, operation);
      },
      dataset,
      options,
      "follows",
    );
  }

  /**
   * Writes an Export of one Dataset of a crawl's Run: the Records it committed
   * when the call is made, in number order, in the format named, as its
   * `format` value says: a table (`csv`, `xlsx`, `html_table`, `markdown`, and
   * typed, `parquet`, `avro` and `arrow_ipc`) has one column per field seen in
   * any of its Records, in first-seen order, and one row per Record. Every text
   * Export is UTF-8 with no BOM unless `export.csv.bom` puts one on CSV,
   * characters written as themselves (a character XML 1.0 cannot hold as U+FFFD
   * in XML and feeds) and each value as it was committed: a table cell holds a
   * string as its text, a number, `true` or `false` as JSON writes it, nothing
   * for `null` or a field its Record lacks, and an object or array as its JSON
   * text, or, in CSV under `export.csv.nested: flatten`, an object's fields in
   * dotted columns. A Record whose page was read past a Wall carries its Wall
   * marker, `{kind, vendor?}` as its Provenance holds it, as a `#wall` field in
   * every format but the feeds, where it is a category of the scheme
   * `urn:goodfetch:wall` naming its kind; `wall` set to `true` writes only
   * those Records, `false` only the others. `compression` `gzip` or `zstd`
   * compresses the whole file. Its `settings` and `project` are the run layer
   * and the project configuration its `export.*` settings resolve under; a
   * value the format cannot take fails with `setting_invalid`. Answers the
   * Export, with its Record count, its last Record's number, its size, its
   * SHA-256 and the run epoch of the Run's state it was written from, only once
   * its file is whole in the Run's folder, where `export.save_as` and
   * `export.stream` read it. Records committed after the call starts are not in
   * it. A write the engine's storage refuses fails with `file_write_failed`,
   * side `engine`, leaving no unfinished file, as does a call while that
   * storage is at the `critical` level, before any byte is written. A Dataset
   * the crawl does not declare fails with `dataset_unknown`; an `xlsx` sheet
   * past Excel's limits, with `sheet_capacity_exceeded`.
   *
   * @experimental
   */
  export<I extends object, O>(dataset: Dataset<I, O>, options: ExportOptions): Promise<Export> {
    return exportCrawlDataset(
      this.#crawlHost,
      this.id(),
      (operation) => {
        ensureRunViewOpen(this, operation);
      },
      dataset,
      options,
    );
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
    return readCrawlOutcomes(
      this.#crawlHost,
      this.id(),
      (operation) => {
        ensureRunViewOpen(this, operation);
      },
      options,
    );
  }

  /**
   * Reads from a crawl's committed state the requests whose latest outcome is
   * `failed`, and with `blocked` also `blocked`, each with that outcome:
   * exactly those `run.retry_failed` re-admits with the same option. A skip is
   * a decision and is never listed.
   *
   * @experimental
   */
  failedRequests(options: FailedRequestsOptions = {}): Promise<readonly CrawlRequestOutcome[]> {
    return readCrawlFailedRequests(
      this.#crawlHost,
      this.id(),
      (operation) => {
        ensureRunViewOpen(this, operation);
      },
      options,
    );
  }
}
