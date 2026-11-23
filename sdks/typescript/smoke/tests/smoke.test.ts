// The TypeScript SDKs' smoke scripts (ADR 0069), the `*.mjs` files of the
// folder above, each run by the smoke-script runner on every runtime of the
// job's set.
import { join } from "node:path";
import { describe } from "vitest";
import { smokeTests } from "@goodfetch/test-support/smoke";

describe("the SDKs' smoke scripts", () => {
  smokeTests({ folder: join(import.meta.dirname, "..") });
});
