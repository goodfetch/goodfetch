// Uses each vi method oxlint must refuse (ADR 0062); ci/proofs/run.sh expects
// one `vitest(no-restricted-vi-methods)` error per method.
import { test, vi } from "vitest";

test("uses every refused vi method", async () => {
  vi.mock("node:fs");
  vi.doMock("node:path");
  await vi.importMock("node:os");
  vi.mockObject({});
  vi.spyOn(console, "log");
  vi.stubGlobal("fetch", undefined);
  vi.stubEnv("HOME", "/tmp");
});
