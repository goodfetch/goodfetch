// `@goodfetch/test-support`'s interface: the Vitest fixtures and what they
// are built on (ADR 0062).
export { TestSupportError, type TestSupportErrorCode } from "./errors.ts";
export { test, type TestSupportFixtures } from "./fixtures.ts";
export {
  ProcessRecord,
  type Exit,
  type RecordedProcess,
  type SpawnOptions,
} from "./process-record.ts";
export { productEnv, type Product } from "./product-env.ts";
export {
  provingGroundOf,
  type ProvingGroundDescription,
  type ProvingGroundFixtures,
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
