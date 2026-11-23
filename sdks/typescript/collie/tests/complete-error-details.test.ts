// Complete error details (ADRs 0092 and 0114), on the Scripted engine as a
// Collie engine: each failure carries every detail its code declares. A
// fill whose one-time code never came raises `SecretUnavailableError` with
// `failure: "no_code"`, and `page_navigated_during_call` raises
// `PageNavigatedDuringCallError`, retryable, naming the document the call
// read and the one that replaced it. Played attached, and as a Daemon,
// whose script exits 0 only once every step played. Python's
// `test_complete_error_details.py` and Rust's `complete_error_details.rs`
// are its twins.
import {
  Collie,
  type GoodfetchError,
  PageError,
  PageNavigatedDuringCallError,
  SecretUnavailableError,
  UnavailableError,
} from "@goodfetch/collie";
import {
  type ClientCaseRecord,
  goodfetchFailure,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** What the script's calls answered. */
interface Played {
  /** The fill of the one-time code no provider made. */
  readonly noCode: GoodfetchError;
  /** The click whose document a navigation it did not start replaced. */
  readonly navigated: GoodfetchError;
}

/** Plays the script's calls on `collie`, then closes it. */
async function played(collie: Collie): Promise<Played> {
  const run = await collie.start();
  const page = await run.newPage();
  const noCode = await goodfetchFailure(page.locator("#otp").fill("%shop_totp%"));
  const navigated = await goodfetchFailure(page.locator("#book").click());
  await run.finish();
  await collie.close();
  return { noCode, navigated };
}

/** The case's checks, the same in both modes. */
async function checkPlayed(clientCase: ClientCaseRecord, answered: Played): Promise<void> {
  const { noCode, navigated } = answered;
  await clientCase.check(
    "a one-time code that never came raises SecretUnavailableError saying no_code",
    () => {
      expect(noCode).toBeInstanceOf(SecretUnavailableError);
      expect(noCode).toBeInstanceOf(UnavailableError);
      expect(noCode).toMatchObject({
        code: "secret_unavailable",
        group: "unavailable",
        retryable: false,
        fix: "Check the TOTP seed of the vault's shop item",
      });
      expect(noCode.details).toEqual({
        placeholder: "shop_totp",
        reference: "bw:shop/totp",
        provider: "bw",
        failure: "no_code",
      });
    },
  );
  await clientCase.check(
    "a navigation the call did not start raises PageNavigatedDuringCallError naming both documents",
    () => {
      expect(navigated).toBeInstanceOf(PageNavigatedDuringCallError);
      expect(navigated).toBeInstanceOf(PageError);
      expect(navigated).toMatchObject({
        code: "page_navigated_during_call",
        group: "page",
        retryable: true,
        url: "https://travel.test/books",
        fix: "Await the Collie call before your driver acts on the page.",
      });
      expect(navigated.details).toEqual({ navigated_to: "https://travel.test/books/1" });
    },
  );
}

describe("A Collie Connection on the Scripted engine", () => {
  test(
    "reads every detail each failure's code declares",
    {
      tags: [
        "client_case/complete-error-details",
        "attached",
        "error:secret_unavailable",
        "error:page_navigated_during_call",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/complete-error-details.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const answered = await played(await Collie.connect({ env }));

      await checkPlayed(clientCase, answered);
    },
  );

  test(
    "reads every detail each failure's code declares from a Daemon",
    {
      tags: [
        "client_case/complete-error-details",
        "daemon",
        "error:secret_unavailable",
        "error:page_navigated_during_call",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/complete-error-details-from-a-daemon.json",
        "socket",
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const answered = await played(await Collie.connect({ env: testEnv.env, engine: "daemon" }));
      const exit = await daemon.exit;

      await checkPlayed(clientCase, answered);
      await clientCase.check(
        "the Daemon played its script through, so the Client sent exactly what it holds",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
