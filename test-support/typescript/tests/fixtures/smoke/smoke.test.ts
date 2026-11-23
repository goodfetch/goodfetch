// The smoke scripts beside this file, as an SDK's tests run its own.
import { join } from "node:path";
import { describe } from "vitest";
import { smokeTests } from "@goodfetch/test-support/smoke";

describe("the fixture's smoke scripts", () => {
  smokeTests({ folder: join(import.meta.dirname, "scripts") });
});
