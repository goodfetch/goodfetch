// `@goodfetch/foxhound` (ADRs 0090, 0095): Foxhound's TypeScript SDK and its `goodfetch-foxhound` bin.
// It re-exports the Collie client package, so a handler's Page is the Page a Collie program gets,
// and adds Foxhound's connection, the generated layer of Foxhound's own entries and the objects only
// Foxhound hands out: crawls and their contexts, Datasets, Records, Exports, Hooks and Politeness.
// Where a name both packages export means more here (`Settings`, `SettingKey`, `TimeoutSettings`,
// `PolitenessSettings`, `SitePreferencesSettings`, `KnownErrorCode`, `ErrorCode`, `ConnectOptions`),
// Foxhound's own export, which adds its keys or values to Collie's, is the one this package gives.
export * from "@goodfetch/collie-client";
export * from "./connection.js";
export * from "./crawl.js";
export * from "./data.js";
export * from "./hooks.js";
export * from "./outcomes.js";
export * from "./output.js";
export * from "./politeness.js";
export * from "./run.js";
export * from "./settings.js";
// Foxhound's own exports of the names it widens, which win over the client package's.
export type { ConnectOptions } from "./connection.js";
export type { ErrorCode, KnownErrorCode } from "./outcomes.js";
export type { PolitenessSettings, SitePreferencesSettings } from "./politeness.js";
export type { SettingKey, Settings, TimeoutSettings } from "./settings.js";
