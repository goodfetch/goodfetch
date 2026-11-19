// ADR 0030's run-time warning for a deprecated or Experimental member (ADR
// 0099), through Node's warning channel. `sample.echo`, the catalog's sample
// entry, is both deprecated and Experimental. Vitest gives each test file its
// own modules, so this file's process has warned of nothing before it.
// Python's `test_run_time_markers.py` and Rust's `markers/tests.rs` are its twins.
import { setImmediate } from "node:timers/promises";
import { expect, test } from "vitest";
import { CollieOperations, type CollieOperationTypes } from "./generated/operations.ts";
import type { Caller } from "./caller.ts";

/** Answers no call: the warnings come before the call is sent. */
const unanswered: Caller<CollieOperationTypes> = {
  call: () => Promise.reject(new Error("unanswered")),
};

test(
  "a deprecated Experimental member warns once of each per process",
  { tags: ["story:11-182", "story:11-184"] },
  async () => {
    const warnings: Error[] = [];
    const listen = (warning: Error): void => {
      warnings.push(warning);
    };
    process.on("warning", listen);
    try {
      const operations = new CollieOperations(unanswered);
      for (const text of ["one", "two"]) {
        // oxlint-disable-next-line typescript/no-deprecated -- the deprecated sample entry is what warns.
        await expect(operations.sampleEcho({ text })).rejects.toThrow("unanswered");
      }
      await setImmediate();
    } finally {
      process.off("warning", listen);
    }

    expect(
      warnings
        .filter((warning) => "code" in warning && String(warning.code).startsWith("GOODFETCH_"))
        .map((warning) => [warning.name, warning.message]),
    ).toEqual([
      [
        "ExperimentalWarning",
        "`sample.echo` is Experimental: outside semver until it is promoted (ADR 0030).",
      ],
      [
        "DeprecationWarning",
        "`sample.echo` is deprecated. Since 0.1: A sample entry, which the real catalog replaces.",
      ],
    ]);
  },
);
