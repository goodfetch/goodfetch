// Points in time in results on the Scripted engine as the Attached engine
// (ADRs 0069, 0092 and 0099): the RFC 3339 text the engine sends, with any
// offset, reaches the program as a `Date` in UTC, to the millisecond.
import { Collie } from "@goodfetch/collie";
import { scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("Points in time on the Scripted engine as the Attached engine", () => {
  test(
    "reads a Run report's points in time as Dates in UTC, a finer part than milliseconds dropped",
    { tags: ["attached", "story:11-91"] },
    async ({ testEnv }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/points-in-time.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });

      const report = await (await collie.start()).finish();

      expect(report.started_at).toBeInstanceOf(Date);
      expect(report.started_at.toISOString()).toBe("2026-11-19T12:30:00.123Z");
      expect(report.ended_at.toISOString()).toBe("2026-11-19T12:31:00.500Z");
    },
  );
});
