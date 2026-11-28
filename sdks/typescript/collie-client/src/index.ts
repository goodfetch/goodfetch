// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// this Collie client package, `@goodfetch/collie-client`, which it
// re-exports, holds `Run`, `Session`, `Page` and `Locator`,
// with the results and errors; `@goodfetch/foxhound` holds `Foxhound`,
// `Crawl`, `Label`, `Dataset` and `CrawlContext`.
//
// `@goodfetch/collie-client` (ADR 0095): the values every Product's
// Connection hands out, a Foxhound program's included, the error classes of
// Collie's Error codes and the Connection both Products' types implement.
// Programs import it through their Product's package; a library serving both
// Products imports it directly. Importing it reads, patches and logs nothing
// (ADR 0078).
export type { Connection } from "./connection.ts";
export type { EngineInfo, HandlerlessStartOptions, StartOptions } from "./client-connection.ts";
export type { ConnectOptions, ConnectOverOptions } from "./product-connection.ts";
export type { EngineChoice, RemoteDaemon } from "./engine-choice.ts";
export type { Limits, Money } from "./limits.ts";
export type { PageSettings, RunSettings, SessionSettings, Settings } from "./generated/settings.ts";
export type { Logger } from "./logger.ts";
export type { SecretRequest, SecretResolver } from "./secrets.ts";
export { Run } from "./run.ts";
export {
  Checkpoint,
  RunTrace,
  type CheckpointSaveOptions,
  type CheckpointStreamOptions,
  type RunTraceSaveOptions,
  type RunTraceStreamOptions,
} from "./run-copies.ts";
export { RunView } from "./run-view.ts";
export {
  Locator,
  Page,
  Session,
  WebSocket,
  type PageEvents,
  type WebSocketEvents,
} from "./generated/handles.ts";
export type { Listener, Subscription } from "./handle.ts";
export type { Handle, RawCallOptions, RawParams } from "./raw-call.ts";
export type { Sequence } from "./sequence.ts";
export {
  KeyValueStore,
  KeyValueStoreView,
  type KeyValueStoreKeysOptions,
  type StoredBytes,
  type StoredValue,
} from "./key-value-store.ts";
export {
  Schedule,
  Schedules,
  type CreatedSchedule,
  type ScheduleCallOptions,
  type SchedulePreviewOptions,
  type ScheduleWriteOptions,
} from "./schedules.ts";
export { Response } from "./response.ts";
export type { JSONObject, JSONValue } from "./json.ts";
export type {
  JSONSchema,
  Schema,
  SchemaInput,
  SchemaOutput,
  StandardJSONSchemaV1,
  StandardSchemaV1,
  TypedSchema,
} from "./schema.ts";
export * from "./generated/errors.ts";
export { AIWorkResult, type AIWorkFields, type Ended, type PartialOf } from "./ai-work.ts";
export {
  verifyWebhook,
  type VerifyWebhookOptions,
  type WebhookBody,
  type WebhookHeaders,
} from "./webhook.ts";
export type { FieldValue, FieldValues } from "./field-values.ts";
export type {
  Acted,
  BlockCause,
  CallOptions,
  CatalogTarget,
  CheckpointSaved,
  Claim,
  Completeness,
  ConsoleMessage,
  ExplainedSetting,
  FormatsRequest,
  FullNotification,
  Lifecycle,
  Navigated,
  NotificationData,
  NotificationEvent,
  NotificationPayload,
  PageClosed,
  PageExchange,
  PageField,
  PageFired,
  PageFormats,
  PagePopup,
  PageRead,
  PageRequest,
  PageResponse,
  PageWebSocket,
  Product,
  Recording,
  RecordingMode,
  RecordingSaved,
  Reported,
  RunOrigin,
  RunProgress,
  RunReport,
  SessionClosed,
  SettingsExplanation,
  SettingsFile,
  SettingsLayer,
  StartedAs,
  StopReason,
  StoppedBy,
  ThinNotification,
  TraceSaved,
  UnreadField,
  UnreadReason,
  Verdict,
  WebError,
  WebhookInvalidDetails,
  WebhookInvalidReason,
  WebSocketClosed,
  WebSocketFrame,
} from "./generated/index.ts";
