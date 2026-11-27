// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// the Collie client package this package re-exports,
// `@goodfetch/collie-client`, holds `Run`, `Session`, `Page` and `Locator`,
// with the results and errors; this package, `@goodfetch/foxhound`, holds
// `Foxhound`, `Crawl`, `Label`, `Dataset` and `CrawlContext`.
//
// `@goodfetch/foxhound` (ADRs 0090 and 0095): Foxhound's TypeScript SDK. It
// re-exports the Collie client package, so a Foxhound program's Run, results
// and error classes are the ones a Collie program holds, and adds the
// Connection, which finds Foxhound's executable and reads only
// `GOODFETCH_FOXHOUND_ENGINE_PATH`, the crawl a program declares and starts
// (ADR 0093), and the classes of Foxhound's own Error codes, each under a
// group class of the client package. It depends on no Collie package but the
// client package. Importing it reads, patches and logs nothing (ADR 0078).
export * from "@goodfetch/collie-client";
export * from "./generated/errors.ts";
// Foxhound's values of the types both Products share, which replace the
// client package's names of the same types (ADR 0095).
export type {
  AuditKind,
  BlockCause,
  CompletenessReasonCode,
  LimitScope,
  RunWarningCode,
  UsageKind,
} from "./generated/shared-types.ts";
export {
  Foxhound,
  type CodeFreeCrawl,
  type FoxhoundConnectOptions,
  type FoxhoundConnectOverOptions,
  type FoxhoundHandlerlessStartOptions,
  type FoxhoundLimits,
  type FoxhoundStartOptions,
  type StartCodeFreeCrawlOptions,
  type StartCrawlOptions,
} from "./foxhound.ts";
export {
  Crawl,
  Dataset,
  Label,
  seed,
  sitemap,
  type AttemptRun,
  type CrawlContext,
  type DatasetOptions,
  type DatasetRecord,
  type DatasetSchemaOptions,
  type EnqueueLinksOptions,
  type Handler,
  type LabelOptions,
  type LinkPayload,
  type LinksPayload,
  type PayloadInput,
  type PayloadSchema,
  type PayloadThen,
  type RecordInput,
  type RecordOutput,
  type RecordSchema,
  type RequestOptions,
} from "./crawl.ts";
export {
  CrawlRun,
  type FailedRequestsOptions,
  type OutcomesOptions,
  type RecordsOptions,
  type RejectedRecordsOptions,
  type RetryFailedOptions,
} from "./crawl-run.ts";
export {
  Export,
  type ExportOptions,
  type ExportSaveOptions,
  type ExportStreamOptions,
} from "./export.ts";
export type {
  BackendRequest,
  BackendResponse,
  FetchBackendHandler,
  SinkDelivery,
  SinkHandler,
} from "./handlers.ts";
export type {
  AttemptError,
  CrawlDeclaration,
  CrawlExtraction,
  CrawlRequestOutcome,
  CrawlRunProgress,
  CrawlRunReport,
  CrawlSettings,
  DatasetRecords,
  Enqueued,
  EnqueuedLink,
  EnqueuedLinks,
  ExportCompression,
  ExportFormat,
  ExportId,
  ExportSaved,
  FoxhoundSettings,
  LabelSettings,
  MatchedLink,
  NothingCommitted,
  OutcomeKind,
  OutcomeRequest,
  OutcomesAfter,
  RecordIssue,
  RejectedRecord,
  RequestCounts,
  RequestProgress,
  RequestSeed,
  Seed,
  SitemapRule,
  SitemapSeed,
  SkipReason,
} from "./generated/index.ts";
