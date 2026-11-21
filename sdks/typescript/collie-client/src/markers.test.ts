// ADR 0030's run-time warning for a deprecated or Experimental entry (ADR
// 0099): once per process per name and marker, through Node's warning
// channel. Python's `tests/integration/test_run_time_markers.py` and Rust's
// `markers/tests.rs` are its twins.
import { setImmediate } from "node:timers/promises";
import { expect, test } from "vitest";
import { type EntryMarkers, warnDeprecated, warnEntry, warnExperimental } from "./markers.ts";

/** The `[name, message]` of each warning the SDK emits while `act` runs. */
async function emitted(act: () => void): Promise<[string, string][]> {
  const warnings: Error[] = [];
  const listen = (warning: Error): void => {
    warnings.push(warning);
  };
  process.on("warning", listen);
  try {
    act();
    await setImmediate();
  } finally {
    process.off("warning", listen);
  }
  return warnings
    .filter((warning) => "code" in warning && String(warning.code).startsWith("GOODFETCH_"))
    .map((warning) => [warning.name, warning.message]);
}

test("a deprecated entry warns once per process through process.emitWarning", async () => {
  const warnings = await emitted(() => {
    warnDeprecated("fixture.old", "Since 0.1: use `fixture.new`.");
    warnDeprecated("fixture.old", "Since 0.1: use `fixture.new`.");
  });

  expect(warnings).toEqual([
    ["DeprecationWarning", "`fixture.old` is deprecated. Since 0.1: use `fixture.new`."],
  ]);
});

test(
  "an Experimental entry warns once per process through process.emitWarning",
  { tags: ["story:11-184"] },
  async () => {
    const warnings = await emitted(() => {
      warnExperimental("fixture.trial");
      warnExperimental("fixture.trial");
    });

    expect(warnings).toEqual([
      [
        "ExperimentalWarning",
        "`fixture.trial` is Experimental: outside semver until it is promoted (ADR 0030).",
      ],
    ]);
  },
);

test("each marker of one entry warns once", async () => {
  const warnings = await emitted(() => {
    warnExperimental("fixture.both");
    warnDeprecated("fixture.both", "Since 0.1: gone.");
    warnExperimental("fixture.both");
    warnDeprecated("fixture.both", "Since 0.1: gone.");
  });

  expect(warnings.map(([name]) => name)).toEqual(["ExperimentalWarning", "DeprecationWarning"]);
});

test("a hand-written member warns of each marker its entry's generated markers give, once", async () => {
  const entries: Readonly<Record<"fixture.served" | "fixture.stable", EntryMarkers>> = {
    "fixture.served": {
      experimental: true,
      deprecated: "Since 0.1, removed no earlier than 1.0: gone.",
    },
    "fixture.stable": { experimental: false, deprecated: undefined },
  };

  const warnings = await emitted(() => {
    warnEntry(entries, "fixture.served");
    warnEntry(entries, "fixture.stable");
    warnEntry(entries, "fixture.served");
  });

  expect(warnings).toEqual([
    [
      "ExperimentalWarning",
      "`fixture.served` is Experimental: outside semver until it is promoted (ADR 0030).",
    ],
    [
      "DeprecationWarning",
      "`fixture.served` is deprecated. Since 0.1, removed no earlier than 1.0: gone.",
    ],
  ]);
});
