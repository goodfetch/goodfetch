// Limits in code (ADR 0080): a program caps its Client connection at
// `connect()` and a Run at its start, with a price as an exact `Money`, and
// each only lowers what covers that work. The cases play on the Scripted
// engine, which checks every message the SDK sends; the last test runs the
// same calls on an Attached engine of Collie's executable of the same commit.
import {
  Collie,
  LimitRaiseRefusedError,
  LimitReachedError,
  SettingInvalidError,
  type Logger,
} from "@goodfetch/collie";
import { failure, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("Limits in code on the Scripted engine as the Attached engine", () => {
  test(
    "sends a connection's Limits after hello, and a Run's in its start, a price in millionths",
    { tags: ["client_case/limits-sent-in-millionths", "attached", "story:11-51"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/limits-sent-in-millionths.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({
        env,
        logger,
        limits: { modelCalls: 100, price: { paidSolves: { amount: "2.50", currency: "USD" } } },
      });
      const run = await collie.start({
        runId: "capped",
        limits: {
          browserTime: 60_000,
          price: { proxyBytes: { amount: "0.000001", currency: "EUR" } },
        },
      });

      const report = await run.finish();
      await collie.close();

      await clientCase.check("the Run started and finished under its Limits", () =>
        expect(report).toMatchObject({ run_id: "capped", lifecycle: "finished" }),
      );
      await clientCase.check(
        "the engine read each Limit as the script expects, so it wrote no failure",
        () => expect(warned).toEqual([]),
      );
    },
  );

  test(
    "refuses a Money it cannot turn exactly into millionths before sending anything",
    { tags: ["client_case/money-inexact-refused-before-sending", "attached", "story:11-51"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/money-inexact-refused.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const finer = await failure(
        collie.start({
          limits: { price: { proxyBytes: { amount: "0.0000001", currency: "USD" } } },
        }),
      );
      const malformed = await failure(
        collie.start({ limits: { price: { modelTokens: { amount: "1,50", currency: "USD" } } } }),
      );
      await collie.close();

      await clientCase.check("an amount finer than a millionth fails with setting_invalid", () => {
        expect(finer).toBeInstanceOf(SettingInvalidError);
        expect(finer).toMatchObject({
          code: "setting_invalid",
          group: "program",
          details: { key: "limits.price.proxyBytes" },
        });
      });
      await clientCase.check("an amount that is no decimal fails with setting_invalid", () =>
        expect(malformed).toMatchObject({
          code: "setting_invalid",
          details: { key: "limits.price.modelTokens" },
        }),
      );
      await clientCase.check("nothing reached the engine past hello", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "fails connect() when the engine refuses one of its Limits, and ends the connection",
    { tags: ["client_case/connect-limit-refused-closes", "attached", "story:11-51"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/limit-refused.json");
      const { logger, warned } = recordingLogger();

      const refused = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({
          env,
          logger,
          limits: { price: { paidSolves: { amount: "2", currency: "USD" } } },
        }),
      );

      await clientCase.check("connect() fails with the engine's refusal", () =>
        expect(refused).toMatchObject({ code: "invalid_params" }),
      );
      await clientCase.check("the engine was ended, so its play wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "lowers a resumed Run's Limit, keeps it at an equal value, and never raises it",
    { tags: ["client_case/limits-lowered-never-raised", "attached", "story:11-51"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/limits-lowered-never-raised.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const startedAs: string[] = [];
      for (const requests of [5, 3, 5]) {
        const run = await collie.start({ runId: "nightly", limits: { requests } });
        startedAs.push(run.startedAs());
        await run.close();
      }
      const raised = await failure(collie.start({ runId: "nightly", limits: { requests: 8 } }));
      await collie.close();

      await clientCase.check("each start below or at the Limit resumed the Run", () =>
        expect(startedAs).toEqual(["new", "resumed", "resumed"]),
      );
      await clientCase.check("a start above it fails with limit_raise_refused", () => {
        expect(raised).toBeInstanceOf(LimitRaiseRefusedError);
        expect(raised).toMatchObject({
          code: "limit_raise_refused",
          group: "refused",
          retryable: false,
          details: { usage_kind: "requests", given: 8, standing: 5 },
        });
      });
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "fails the call that finds a Limit used up with limit_reached, and the Run ends stopped",
    { tags: ["client_case/limit-reached-stops-the-run", "attached", "story:11-52"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/limit-reached-stops-the-run.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start({
        runId: "capped",
        limits: { price: { proxyBytes: { amount: "1", currency: "USD" } } },
      });
      const page = await run.newPage();

      const reached = await failure(page.goto("https://shop.test/"));
      const report = await run.finish();
      await collie.close();

      await clientCase.check("the call fails with limit_reached, naming the Limit", () => {
        expect(reached).toBeInstanceOf(LimitReachedError);
        expect(reached).toMatchObject({
          code: "limit_reached",
          group: "refused",
          details: {
            scope: { kind: "run", id: "capped" },
            usage_kind: "proxy_bytes",
            currency: "USD",
          },
        });
      });
      await clientCase.check("the Run ends stopped and incomplete, for that Limit", () =>
        expect(report).toMatchObject({
          lifecycle: "stopped",
          completeness: "incomplete",
          stop_reason: {
            kind: "limit",
            scope: { kind: "run", id: "capped" },
            usage_kind: "proxy_bytes",
            currency: "USD",
          },
        }),
      );
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );
});

describe("Limits in code on an Attached engine", () => {
  test(
    "holds a connection's Limits and lowers a resumed Run's, refusing a raise",
    { tags: ["story:11-51"] },
    async ({ env }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({
        env,
        limits: { modelCalls: 100, price: { paidSolves: { amount: "2.50", currency: "USD" } } },
      });

      const startedAs: string[] = [];
      for (const requests of [5, 3, 5]) {
        const run = await collie.start({ runId: "resumed", limits: { requests } });
        startedAs.push(run.startedAs());
        await run.close();
      }
      const raised = await failure(collie.start({ runId: "resumed", limits: { requests: 8 } }));

      expect({ startedAs, raised }).toMatchObject({
        startedAs: ["new", "resumed", "resumed"],
        raised: { code: "limit_raise_refused", details: { usage_kind: "requests", given: 8 } },
      });
    },
  );
});

/** A host logger that keeps each warning, where the engine's stderr lines go. */
function recordingLogger(): { readonly logger: Logger; readonly warned: string[] } {
  const warned: string[] = [];
  const logger: Logger = {
    // It asks for none of the engine's log events: `hello` says `off`, as the script expects.
    level: "off",
    debug: () => {},
    info: () => {},
    warn: (message) => {
      warned.push(message);
    },
    error: () => {},
  };
  return { logger, warned };
}
