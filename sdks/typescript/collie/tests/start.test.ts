// A start reaches the Run its Run ID names (ADRs 0078 and 0102): a fresh ID
// begins it, an interrupted Run's resumes it and a finished Run's returns it
// ended, each start the engine refuses raising its generated class. The case
// plays on the Scripted engine, which checks every message the SDK sends;
// the last test runs a start's answers on an Attached engine of Collie's
// executable of the same commit, with the Run ID off its pattern that no
// script may send.
import {
  Collie,
  LimitReachedError,
  RunConflictError,
  RunEndedError,
  RunIdInvalidError,
  RunInUseError,
  type Logger,
} from "@goodfetch/collie";
import { failure, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("A start on the Scripted engine as the Attached engine", () => {
  test(
    "reaches the Run its Run ID names, logs a resume and an ended Run, and raises each refusal's class",
    {
      tags: [
        "client_case/start-reaches-its-run",
        "attached",
        "story:11-126",
        "story:11-127",
        "operation:run.started_as",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/start-reaches-its-run.json");
      const { logger, logged, stderr } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const begun = await collie.start({ runId: "nightly", tags: ["shop", "eu"] });
      const id = begun.id();
      await begun.close();
      const resumed = await collie.start({ runId: "nightly" });
      await resumed.finish();
      const ended = await collie.start({ runId: "nightly" });
      const noWork = await failure(ended.newPage());
      const inUse = await failure(collie.start({ runId: "busy" }));
      const conflict = await failure(collie.start({ runId: "crawled" }));
      const reached = await failure(collie.start({ runId: "capped" }));
      await collie.close();

      await clientCase.check(
        "run.startedAs() answers how each start reached the Run, without a message to the engine",
        () =>
          expect([begun, resumed, ended].map((run) => run.startedAs())).toEqual([
            "new",
            "resumed",
            "ended",
          ]),
      );
      await clientCase.check("run.id() names the Run without a message to the engine", () =>
        expect(id).toBe("nightly"),
      );
      await clientCase.check(
        "the resume is logged at info, the ended Run at warning, through the host's logger",
        () =>
          expect(logged).toEqual([
            [
              "info",
              "The start resumed the Run `nightly` from its committed state.",
              { runId: "nightly", startedAs: "resumed" },
            ],
            [
              "warn",
              "The start found the Run `nightly` ended: it is returned as it ended and takes no new work.",
              { runId: "nightly", startedAs: "ended" },
            ],
          ]),
      );
      await clientCase.check("a call that would give the ended Run work raises run_ended", () => {
        expect(noWork).toBeInstanceOf(RunEndedError);
        expect(noWork).toMatchObject({
          code: "run_ended",
          group: "program",
          runId: "nightly",
        });
      });
      await clientCase.check("a Run another engine writes raises run_in_use", () => {
        expect(inUse).toBeInstanceOf(RunInUseError);
        expect(inUse).toMatchObject({
          code: "run_in_use",
          retryable: true,
          runId: "busy",
        });
      });
      await clientCase.check("a Run of other inputs raises run_conflict listing each", () => {
        expect(conflict).toBeInstanceOf(RunConflictError);
        expect(conflict).toMatchObject({
          code: "run_conflict",
          runId: "crawled",
          details: { differences: ["product", "kind", "caller", "seeds"] },
        });
      });
      await clientCase.check("a Run whose Limit is used up raises limit_reached", () => {
        expect(reached).toBeInstanceOf(LimitReachedError);
        expect(reached).toMatchObject({
          code: "limit_reached",
          details: {
            scope: { kind: "run", id: "capped" },
            usage_kind: "requests",
            resets_at: "2026-02-01T00:00:00Z",
          },
        });
      });
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(stderr).toEqual([]),
      );
    },
  );
});

describe("A start on an Attached engine", () => {
  test(
    "begins, refuses a live Run, resumes, returns a finished Run ended, and refuses tags past their bounds",
    { tags: ["story:11-126", "story:11-127"] },
    async ({ env }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });

      const begun = await collie.start({ runId: "held", tags: ["shop"] });
      const inUse = await failure(collie.start({ runId: "held" }));
      await begun.close();
      const resumed = await collie.start({ runId: "held" });
      await resumed.finish();
      const ended = await collie.start({ runId: "held" });
      const invalid = await failure(collie.start({ runId: "Held" }));
      const tooMany = await failure(collie.start({ tags: Array.from({ length: 17 }, () => "t") }));
      const tooLong = await failure(collie.start({ tags: ["t".repeat(65)] }));

      expect(invalid).toBeInstanceOf(RunIdInvalidError);
      expect({
        startedAs: [begun, resumed, ended].map((run) => run.startedAs()),
        inUse,
        invalid,
        tooMany,
        tooLong,
      }).toMatchObject({
        startedAs: ["new", "resumed", "ended"],
        inUse: { code: "run_in_use", runId: "held" },
        invalid: { code: "run_id_invalid", details: { pattern: expect.any(String) } },
        tooMany: { code: "invalid_params" },
        tooLong: { code: "invalid_params" },
      });
    },
  );
});

type Logged = readonly [level: "info" | "warn", message: string, fields: unknown];

/**
 * A host logger that keeps what the SDK logs at info and warning, and apart
 * the engine's stderr lines, which a failed play writes. It leaves out the
 * engine `connect()` reached, logged at info with its `EngineInfo`, which
 * engine-choice.test.ts checks.
 */
function recordingLogger(): {
  readonly logger: Logger;
  readonly logged: Logged[];
  readonly stderr: string[];
} {
  const logged: Logged[] = [];
  const stderr: string[] = [];
  const logger: Logger = {
    // It asks for none of the engine's log events: `hello` says `off`, as the script expects.
    level: "off",
    debug: () => {},
    info: (message, fields) => {
      if (fields?.["engine"] === undefined) {
        logged.push(["info", message, fields]);
      }
    },
    warn: (message, fields) => {
      if (fields?.["source"] === "engine stderr") {
        stderr.push(message);
      } else {
        logged.push(["warn", message, fields]);
      }
    },
    error: () => {},
  };
  return { logger, logged, stderr };
}
