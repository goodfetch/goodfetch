// Errors and outcomes (ADR 0079), on the Scripted engine as a Collie
// engine: a navigation to a missing page returns its 404 response, an
// outcome, where each failure raises its code's class under its group's,
// a code no catalog has raising its group's class with the code kept, and
// carries its code, group, retryable flag, operation, call and Run IDs, the
// fields that apply and its typed details. The program fills a field with
// a Secret's value, the canary (ADR 0067): no rendering of any error, its
// message, string form, inspected form or stack, nor anything the SDK
// logged, holds it. Played attached, and as a Daemon, whose script exits 0
// only once every step played.
import { inspect } from "node:util";
import {
  Collie,
  DeadlineExceededError,
  EnginePanickedError,
  type GoodfetchError,
  InternalError,
  type Logger,
  ModelError,
  PageError,
  ProgramError,
  RefusedError,
  SecretInputRefusedError,
  SecretUnavailableError,
  SecretUnboundError,
  TargetNotUniqueError,
  UnavailableError,
} from "@goodfetch/collie";
import {
  type Canary,
  type ClientCaseRecord,
  goodfetchFailure,
  scriptedEngineEnv,
  searchCanaries,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The Run the script starts. */
const RUN_ID = "01ke6p4ym0j8x3v5wq2rtbn7hc";

/** The Secret's value the program fills `#card` with, which no error may render. */
const CANARY: Canary = {
  id: "card-typed-password",
  kind: "typed_password",
  value: "typed-password-canary-3f9b2d71c8e0",
};

/** The timeout the script's page failures outlived, set by the call's argument. */
const ACTION_TIMEOUT = { key: "timeout.action", value: 5000, layer: "run", origin: "argument" };

/**
 * A host logger at `debug` that keeps every line the SDK logs, at any
 * level, with its fields, and each warning apart, where the engine's stderr
 * lines go.
 */
function loggingEverything(): {
  readonly logger: Logger;
  readonly logged: string[];
  readonly warned: string[];
} {
  const logged: string[] = [];
  const warned: string[] = [];
  function keep(message: string, fields?: Readonly<Record<string, unknown>>): void {
    logged.push(`${message} ${inspect(fields, { depth: null })}`);
  }
  const logger: Logger = {
    level: "debug",
    debug: keep,
    info: keep,
    warn: (message, fields) => {
      keep(message, fields);
      warned.push(message);
    },
    error: keep,
  };
  return { logger, logged, warned };
}

/** What the case's calls answered, in the script's order. */
interface Played {
  readonly missing: unknown;
  readonly unbound: GoodfetchError;
  readonly inputRefused: GoodfetchError;
  readonly notUnique: GoodfetchError;
  readonly deadline: GoodfetchError;
  readonly unavailable: GoodfetchError;
  readonly fromOutside: GoodfetchError;
  readonly panicked: GoodfetchError;
}

/** Plays the case's calls on `collie`, then finishes the Run and closes it. */
async function played(collie: Collie): Promise<Played> {
  const run = await collie.start();
  const page = await run.newPage();
  const missing = await page.goto("https://shop.test/missing");
  const unbound = await goodfetchFailure(page.locator("#user").fill("%shop_user%"));
  const inputRefused = await goodfetchFailure(page.locator("#password").fill("%shop_password%"));
  const notUnique = await goodfetchFailure(page.locator("#buy").click());
  const deadline = await goodfetchFailure(page.locator("#card").fill(CANARY.value));
  const unavailable = await goodfetchFailure(page.locator("#otp").fill("%shop_otp%"));
  const fromOutside = await goodfetchFailure(page.locator("#next").click());
  const panicked = await goodfetchFailure(page.locator("#done").click());
  await run.finish();
  await collie.close();
  return {
    missing,
    unbound,
    inputRefused,
    notUnique,
    deadline,
    unavailable,
    fromOutside,
    panicked,
  };
}

/**
 * Checks the 404 returned and each failure's class, code, group and
 * details, and that no rendering of an error nor any line in `logged`
 * holds the canary.
 */
async function checkPlayed(
  clientCase: ClientCaseRecord,
  {
    missing,
    unbound,
    inputRefused,
    notUnique,
    deadline,
    unavailable,
    fromOutside,
    panicked,
  }: Played,
  logged: readonly string[],
): Promise<void> {
  const errors: readonly GoodfetchError[] = [
    unbound,
    inputRefused,
    notUnique,
    deadline,
    unavailable,
    fromOutside,
    panicked,
  ];
  await clientCase.check("a 404 is the navigation's outcome, returned, not raised", () => {
    expect(missing).toMatchObject({ status: 404, url: "https://shop.test/missing" });
  });
  await clientCase.check("each failure carries its operation, call ID and Run ID", () => {
    expect(errors.map(({ operation, callId, runId }) => [operation, callId, runId])).toEqual([
      ["locator.fill", "call-1", RUN_ID],
      ["locator.fill", "call-2", RUN_ID],
      ["locator.click", "call-3", RUN_ID],
      ["locator.fill", "call-4", RUN_ID],
      ["locator.fill", "call-5", RUN_ID],
      ["locator.click", "call-6", RUN_ID],
      ["locator.click", "call-7", RUN_ID],
    ]);
  });
  await clientCase.check(
    "an unbound placeholder raises SecretUnboundError, a ProgramError naming the placeholders bound",
    () => {
      expect(unbound).toBeInstanceOf(SecretUnboundError);
      expect(unbound).toBeInstanceOf(ProgramError);
      expect(unbound).toMatchObject({
        code: "secret_unbound",
        group: "program",
        retryable: false,
        target: "#user",
        url: "https://shop.test/login",
        fix: "Bind `shop_user` to a Secret reference in the Run's `secrets`, or use a placeholder it binds",
      });
      expect(unbound.details).toEqual({
        placeholder: "shop_user",
        bound: ["shop_password", "shop_otp"],
      });
    },
  );
  await clientCase.check(
    "a refused input raises SecretInputRefusedError, a RefusedError naming the Secret's reference",
    () => {
      expect(inputRefused).toBeInstanceOf(SecretInputRefusedError);
      expect(inputRefused).toBeInstanceOf(RefusedError);
      expect(inputRefused).toMatchObject({
        code: "secret_input_refused",
        group: "refused",
        retryable: false,
        target: "#password",
      });
      expect(inputRefused.details).toEqual({
        placeholder: "shop_password",
        reference: "env:SHOP_PASSWORD",
        domain: "shop.test",
        reason: "invalid_certificate_exception",
      });
    },
  );
  await clientCase.check(
    "a locator matching two elements raises TargetNotUniqueError, a PageError with its limit and call log",
    () => {
      expect(notUnique).toBeInstanceOf(TargetNotUniqueError);
      expect(notUnique).toBeInstanceOf(PageError);
      expect(notUnique).toMatchObject({
        code: "target_not_unique",
        group: "page",
        retryable: false,
        target: "#buy",
        url: "https://shop.test/cart",
        limit: ACTION_TIMEOUT,
        callLog: ["waiting for #buy", "#buy matched 2 elements"],
      });
      expect(notUnique.details).toEqual({
        matches: [
          {
            element: '<button id="buy">Buy now</button>',
            locator: "getByRole('button', { name: 'Buy now' })",
          },
          {
            element: '<button id="buy">Buy later</button>',
            locator: "getByRole('button', { name: 'Buy later' })",
          },
        ],
        elapsed: 5000,
      });
    },
  );
  await clientCase.check(
    "an outlived timeout raises DeadlineExceededError, retryable, saying what it observed and which limit",
    () => {
      expect(deadline).toBeInstanceOf(DeadlineExceededError);
      expect(deadline).toMatchObject({
        code: "deadline_exceeded",
        group: "deadline_exceeded",
        retryable: true,
        target: "#card",
        observed: "the field stayed disabled",
        limit: ACTION_TIMEOUT,
        fix: "Wait for #card to be enabled first, or raise the call's timeout",
      });
      expect(deadline.details).toEqual({ elapsed: 5000, waited_for: "target_editable" });
    },
  );
  await clientCase.check(
    "a Secret with no value raises SecretUnavailableError, an UnavailableError naming its provider",
    () => {
      expect(unavailable).toBeInstanceOf(SecretUnavailableError);
      expect(unavailable).toBeInstanceOf(UnavailableError);
      expect(unavailable).toMatchObject({
        code: "secret_unavailable",
        group: "unavailable",
        retryable: false,
        fix: "Set SHOP_OTP in the engine's environment",
      });
      expect(unavailable.details).toEqual({
        placeholder: "shop_otp",
        reference: "env:SHOP_OTP",
        provider: "env",
        failure: "missing",
      });
    },
  );
  await clientCase.check("a code no catalog has raises its group's class, the code kept", () => {
    expect(fromOutside).toBeInstanceOf(ModelError);
    expect(fromOutside).toMatchObject({
      code: "model_output_invalid",
      group: "model",
      retryable: true,
      target: "#next",
    });
    expect(fromOutside.details).toEqual({ attempts: 2 });
  });
  await clientCase.check("an engine panic raises EnginePanickedError, an InternalError", () => {
    expect(panicked).toBeInstanceOf(EnginePanickedError);
    expect(panicked).toBeInstanceOf(InternalError);
    expect(panicked).toMatchObject({
      code: "engine_panicked",
      group: "internal",
      retryable: false,
    });
    expect(panicked.details).toEqual({
      ended: "call",
      id: "panic-1",
      crash_report: "/srv/goodfetch/crashes/panic-1.json",
    });
  });
  await clientCase.check(
    "no error's message, string form, inspected form or stack, nor any log line, holds the Secret",
    () => {
      const rendered = errors.flatMap((error, index) => [
        { name: `message ${index}`, text: error.message },
        { name: `string ${index}`, text: String(error) },
        { name: `inspected ${index}`, text: inspect(error, { depth: null }) },
        { name: `stack ${index}`, text: error.stack ?? "" },
      ]);
      const logLines = logged.map((line, index) => ({ name: `log ${index}`, text: line }));
      expect(searchCanaries([CANARY], [...rendered, ...logLines])).toEqual([]);
    },
  );
}

describe("A Collie Connection on the Scripted engine", () => {
  test(
    "returns an outcome as a value and raises each failure as its code's class, rendering no Secret",
    {
      tags: [
        "client_case/errors-and-outcomes",
        "attached",
        "story:11-28",
        "story:11-30",
        "story:11-187",
        "error:secret_input_refused",
        "error:target_not_unique",
        "error:deadline_exceeded",
        "error:secret_unavailable",
        "error:engine_panicked",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/errors-and-outcomes.json");
      const { logger, logged, warned } = loggingEverything();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const answered = await played(await Collie.connect({ env, logger }));

      await checkPlayed(clientCase, answered, logged);
      await clientCase.check("the script played to its end, so its engine wrote no failure", () => {
        expect(warned).toEqual([]);
      });
    },
  );

  test(
    "returns the same outcome and raises the same failures from a Daemon, rendering no Secret",
    {
      tags: [
        "client_case/errors-and-outcomes",
        "daemon",
        "story:11-28",
        "story:11-30",
        "story:11-187",
        "error:secret_input_refused",
        "error:target_not_unique",
        "error:deadline_exceeded",
        "error:secret_unavailable",
        "error:engine_panicked",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/errors-and-outcomes-from-a-daemon.json",
        "socket",
      );
      const { logger, logged } = loggingEverything();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: testEnv.env, engine: "daemon", logger });
      const answered = await played(collie);
      const exit = await daemon.exit;

      await checkPlayed(clientCase, answered, logged);
      await clientCase.check(
        "the Daemon played its script through, so the Client sent exactly what it holds",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
