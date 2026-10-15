// Foxhound's connection (ADRs 0090, 0095): `Foxhound` carries every member of `Collie` under the same
// names, Collie's Run start included, plus its own, so moving a program from Collie to Foxhound
// changes its import and its connection type only. It finds Foxhound's executable through its own
// platform packages and reads only `GOODFETCH_FOXHOUND_ENGINE_PATH` (ADR 0090).

import type {
  CallOptions,
  ConnectOptions as CollieConnectOptions,
  Connection,
  EngineInfo,
  ExplainedSetting,
  Identities,
  JSONObject,
  JSONValue,
  Run,
  RunView,
  StartOptions,
} from "@goodfetch/collie-client";
import type { Crawl, CrawlRun, CrawlRunView, CrawlStartOptions } from "./crawl.js";
import type { FetchBackendHandler, SinkHandler } from "./hooks.js";
import type { SettingKey, Settings } from "./settings.js";

/** Options of `Foxhound.connect()`: Collie's, Foxhound's settings, and the SDK handlers of Foxhound's own Plugin kinds (ADR 0088). */
export interface ConnectOptions extends Omit<CollieConnectOptions, "settings"> {
  /** Settings for this connection and everything opened beneath it, Foxhound's keys included. */
  readonly settings?: Settings;
  /**
   * Fetch backends answered by the program's own code, by the name settings choose them by (ADR
   * 0104; ADR 0033's SDK-handler form). The only one on the connection becomes the value of the
   * setting that chooses a Fetch backend, with the run-layer origin `registered`, which every other
   * run-layer value replaces and a lock beats; two fail `connect()` when nothing else sets the key.
   */
  readonly fetchBackends?: Readonly<Record<string, FetchBackendHandler>>;
  /**
   * Sinks answered by the program's own code, each serving the `sinks.<name>` entry of its name,
   * which then takes no `type` (ADRs 0103, 0104).
   */
  readonly sinks?: Readonly<Record<string, SinkHandler>>;
}

/**
 * A connection to a Foxhound engine: the entry point (ADR 0078). A Collie engine refuses it at
 * `connect()` with `not_in_product` (ADR 0095). Each one is independent, and many may be open in
 * one process.
 */
export declare class Foxhound implements Connection {
  private constructor();
  /** Starts or finds the engine and completes `hello` before returning. */
  static connect(options?: ConnectOptions): Promise<Foxhound>;
  engine(): EngineInfo;
  /** The install's Saved identities (ADR 0092). */
  readonly identities: Identities;
  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.
   *
   * Collie's Run start (ADR 0102), unchanged: a Run with no crawl. Fails with `model_price_unknown` when a currency Limit or `ai.allowance.price` applies and the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).
   */
  start(options?: StartOptions): Promise<Run>;
  openRun(runId: string, options?: CallOptions): Promise<RunView>;
  /**
   * Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.
   *
   * Starts a Run of the crawl from its seeds and returns once it is started; its handlers run from
   * then on (ADR 0102, Firecrawl's `start_crawl`). A `runId` naming an existing Run behaves as
   * `start`'s does, the seeds and the Plan file being inputs that must match. Throws
   * `LabelUnhandledError` when a Label has no handler, and `LabelUnknownError` when a resumed Run
   * has pending requests of a Label no longer declared (ADR 0093). Fails with `model_price_unknown` when a currency Limit or `ai.allowance.price` applies and the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).
   */
  startCrawl(crawl: Crawl, options: CrawlStartOptions): Promise<CrawlRun>;
  /** Opens a crawl's Run by its Run ID to read it, as `openRun` does, with its outcomes, Records and Exports. */
  openCrawlRun(runId: string, options?: CallOptions): Promise<CrawlRunView>;
  /** Cancels in-flight calls, closes every Run it opened and ends every borrow; stops an Attached engine, only disconnects from a Daemon. */
  close(options?: CallOptions): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
  explainSettings(keys?: readonly SettingKey[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
  /** The raw catalog call (ADR 0099): `params` names every value itself. */
  call(operation: string, params?: JSONObject, options?: CallOptions): Promise<JSONValue>;
}
