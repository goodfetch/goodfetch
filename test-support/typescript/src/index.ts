// `@goodfetch/test-support`'s interface: the Vitest fixtures and what they
// are built on (ADR 0062).
export { TestSupportError, type TestSupportErrorCode } from "./errors.ts";
export { test, type TestSupportFixtures } from "./fixtures.ts";
export { locales, timeZones, type TimeZoneCase } from "./locales.ts";
export {
  ProcessRecord,
  type Exit,
  type RecordedProcess,
  type SpawnOptions,
} from "./process-record.ts";
export { productEnv, type Product } from "./product-env.ts";
export { redact, type SnapshotValues } from "./snapshot.ts";
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
