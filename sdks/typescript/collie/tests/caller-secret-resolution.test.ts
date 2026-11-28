// The caller as a Secret provider (ADRs 0067, 0092 and 0104; issue 1281), on
// the Scripted engine as a Collie engine: `connect({ resolveSecret })` answers
// `secrets.resolve` for `caller:<name>` references at the moment of use,
// passing the value straight on with no copy kept or logged; a throw becomes a
// redacted `secret_unavailable` with no Secret canary in any `toString`, error
// message, call log or JSON-RPC error. Played attached and as a Daemon.
import {
  Collie,
  type GoodfetchError,
  type SecretRequest,
  SecretUnavailableError,
  UnavailableError,
} from "@goodfetch/collie";
import {
  type ClientCaseRecord,
  goodfetchFailure,
  recordingLogger,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const CANARY = "canary-caller-secret-value-not-real";

interface Played {
  readonly seen: readonly SecretRequest[];
  readonly failed: GoodfetchError;
  readonly logs: readonly unknown[];
}

async function played(
  open: (
    resolveSecret: (request: SecretRequest) => Promise<string>,
    logger: ReturnType<typeof recordingLogger>["logger"],
  ) => Promise<Collie>,
): Promise<Played> {
  const seen: SecretRequest[] = [];
  const { logger, warned, errored } = recordingLogger();
  const collie = await open(async (request) => {
    seen.push(request);
    if (request.name === "shop_pw") {
      return CANARY;
    }
    throw new Error(`vault read failed for ${CANARY}`);
  }, logger);
  const run = await collie.start();
  const page = await run.newPage();
  await page.locator("#pw").fill("%shop_pw%");
  const failed = await goodfetchFailure(page.locator("#otp").fill("%shop_otp%"));
  await run.finish();
  await collie.close();
  return { seen, failed, logs: [...warned, ...errored] };
}

async function checkPlayed(clientCase: ClientCaseRecord, answered: Played): Promise<void> {
  await clientCase.check(
    "resolveSecret is called once per caller: reference with name, runId and domain",
    () => {
      expect(answered.seen).toEqual([
        { name: "shop_pw", runId: "01ke6p4ym0j8x3v5wq2rtbn7hc", domain: "shop.test" },
        { name: "shop_otp", runId: "01ke6p4ym0j8x3v5wq2rtbn7hc", domain: "shop.test" },
      ]);
    },
  );
  await clientCase.check(
    "a throw becomes SecretUnavailableError with failure resolver_failed and no canary in error, toString, callLog or logs",
    () => {
      const { failed, logs } = answered;
      expect(failed).toBeInstanceOf(SecretUnavailableError);
      expect(failed).toBeInstanceOf(UnavailableError);
      expect(failed).toMatchObject({
        code: "secret_unavailable",
        group: "unavailable",
        retryable: false,
      });
      expect(failed.details).toMatchObject({
        placeholder: "shop_otp",
        reference: "caller:shop_otp",
        provider: "caller",
        failure: "resolver_failed",
      });
      expect(failed.fix).toContain("resolveSecret");
      expect(String(failed)).not.toContain(CANARY);
      expect(failed.message).not.toContain(CANARY);
      expect(JSON.stringify(failed.callLog ?? [])).not.toContain(CANARY);
      expect(JSON.stringify(logs)).not.toContain(CANARY);
    },
  );
}

describe("A Collie Connection with resolveSecret on the Scripted engine", () => {
  test(
    "answers caller: references at the moment of use and redacts failures",
    {
      tags: ["client_case/caller-secret-resolution", "attached", "story:11-98"],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/caller-secret-resolution.json");
      const answered = await played((resolveSecret, logger) =>
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({ env, resolveSecret, logger }),
      );

      await checkPlayed(clientCase, answered);
    },
  );

  test(
    "answers caller: references from a Daemon and redacts failures",
    {
      tags: ["client_case/caller-secret-resolution", "daemon", "story:11-98"],
    },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/caller-secret-resolution-from-a-daemon.json",
        "socket",
      );
      const answered = await played((resolveSecret, logger) =>
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({ env: testEnv.env, engine: "daemon", resolveSecret, logger }),
      );
      const exit = await daemon.exit;

      await checkPlayed(clientCase, answered);
      await clientCase.check(
        "the Daemon played its script through, so no canary reached the JSON-RPC error",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
