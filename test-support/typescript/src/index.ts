// `@goodfetch/test-support`'s interface: the Vitest fixtures and what they
// are built on (ADR 0062).
export {
  searchCanaries,
  type Canary,
  type CanaryHit,
  type CanaryOutput,
  type Encoding,
  type Placement,
  type SecretKind,
} from "./canaries.ts";
export { ClientCaseRecord } from "./case-log.ts";
export { TestSupportError, type TestSupportErrorCode } from "./errors.ts";
export { failure } from "./failure.ts";
export { recordingLogger } from "./recording-logger.ts";
export { test, type TestSupportFixtures } from "./fixtures.ts";
export { locales, timeZones, type TimeZoneCase } from "./locales.ts";
export {
  ProcessRecord,
  type Exit,
  type RecordedProcess,
  type SpawnOptions,
} from "./process-record.ts";
export {
  journey,
  startJourneyRun,
  type FinishedJourneyRun,
  type JourneyRun,
  type JourneyRunStart,
  type TaskSubmission,
} from "./journey.ts";
export {
  journeyDocument,
  writeJourneyDocument,
  type JourneyFacts,
  type JourneyResultDocument,
} from "./journey-document.ts";
export {
  parseCaseList,
  readCaseList,
  type CaseList,
  type DeclaredCapability,
  type DeclaredCase,
} from "./case-list.ts";
export { journeyVectors, type JourneyVariant, type JourneyVector } from "./outcome-vectors.ts";
export {
  playScript,
  type Ending,
  type InMemoryTransport,
  type PlayedMessage,
  type PlayedScript,
} from "./play-script.ts";
export { protocolCounts, type ProtocolCounts } from "./protocol-counts.ts";
export { daemonOf, type DaemonDescription } from "./daemon.ts";
export { productEnv, scriptedEngineEnv, type Product } from "./product-env.ts";
export { redact, type SnapshotValues } from "./snapshot.ts";
export {
  judgeTask,
  provingGroundOf,
  provingGroundRoutes,
  taskStartUrl,
  type TaskResult,
  type ProvingGroundDescription,
  type ProvingGroundFixtures,
  type ProvingGroundRoutes,
  type ProxyCredentials,
} from "./proving-ground.ts";
export {
  startTestEnv,
  toolPath,
  type Program,
  type StartedTestEnv,
  type TestEnvDescription,
  type TestEnvStart,
  type Tools,
} from "./test-env.ts";
export { unicodeRecord, type UnicodeField, type UnicodeRecord } from "./unicode.ts";
