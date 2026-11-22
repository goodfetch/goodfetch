// A `cdp_url` the engine cannot use (ADR 0115), on the Scripted engine:
// `connect()` and the Run's start succeed, and each opening of the default
// Session raises the class of its remedy, under its group, with its
// `retryable` flag, `fix` and details, naming the address and the settings
// layer that gave it.
import {
  CDPURLNotAllowedError,
  CDPURLNotBrowserError,
  CDPURLUnauthorizedError,
  CDPURLUnreachableError,
  Collie,
  ProgramError,
  RefusedError,
  UnavailableError,
} from "@goodfetch/collie";
import { failure, recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("A Collie Connection whose default Session opens from a cdp_url", () => {
  test(
    "connects and starts its Run, and raises each cdp_url code's class when the Session opens",
    {
      tags: [
        "client_case/cdp-url-errors",
        "attached",
        "story:11-294",
        "error:cdp_url_unreachable",
        "error:cdp_url_not_browser",
        "error:cdp_url_unauthorized",
        "error:cdp_url_not_allowed",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/cdp-url-errors.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start();
      const unreachable = await failure(run.newPage());
      const notBrowser = await failure(run.newPage());
      const unauthorized = await failure(run.newPage());
      const notAllowed = await failure(run.newPage());
      await run.finish();
      await collie.close();

      await clientCase.check(
        "a refused connection raises CDPURLUnreachableError, a retryable UnavailableError naming its failure",
        () => {
          expect(unreachable).toBeInstanceOf(CDPURLUnreachableError);
          expect(unreachable).toBeInstanceOf(UnavailableError);
          expect(unreachable).toMatchObject({
            code: "cdp_url_unreachable",
            group: "unavailable",
            retryable: true,
            url: "http://127.0.0.1:9222",
            fix: "Start the browser with remote debugging on that port, or correct the address",
          });
          expect(unreachable).toHaveProperty("details", {
            layer: "user_configuration",
            failure: "connection_refused",
          });
        },
      );
      await clientCase.check(
        "a page target's address raises CDPURLNotBrowserError, a ProgramError naming what answered and the browser's address",
        () => {
          expect(notBrowser).toBeInstanceOf(CDPURLNotBrowserError);
          expect(notBrowser).toBeInstanceOf(ProgramError);
          expect(notBrowser).toMatchObject({
            code: "cdp_url_not_browser",
            group: "program",
            retryable: false,
            url: "ws://127.0.0.1:9222/devtools/page/8C5F0D1E",
            observed: "a CDP target of type `page`",
            fix: "Give the browser's endpoint, ws://127.0.0.1:9222/devtools/browser/1A2B3C4D, which this host's /json/version lists",
          });
          expect(notBrowser).toHaveProperty("details", {
            layer: "user_configuration",
            browser_url: "ws://127.0.0.1:9222/devtools/browser/1A2B3C4D",
          });
        },
      );
      await clientCase.check(
        "a 401 raises CDPURLUnauthorizedError, a RefusedError naming the header and its Secret reference",
        () => {
          expect(unauthorized).toBeInstanceOf(CDPURLUnauthorizedError);
          expect(unauthorized).toBeInstanceOf(RefusedError);
          expect(unauthorized).toMatchObject({
            code: "cdp_url_unauthorized",
            group: "refused",
            retryable: false,
            url: "https://browser.example.com/",
            fix: "Check the Secret references in `headers`",
          });
          expect(unauthorized).toHaveProperty("details", {
            layer: "user_configuration",
            status: 401,
            headers: { Authorization: "env:BROWSER_TOKEN" },
          });
        },
      );
      await clientCase.check(
        "a loopback address from a model's project configuration raises CDPURLNotAllowedError, a RefusedError naming the rule and layer",
        () => {
          expect(notAllowed).toBeInstanceOf(CDPURLNotAllowedError);
          expect(notAllowed).toBeInstanceOf(RefusedError);
          expect(notAllowed).toMatchObject({
            code: "cdp_url_not_allowed",
            group: "refused",
            retryable: false,
            url: "http://127.0.0.1:9222",
            fix: "Set the address in Operator policy or in user configuration, or give a public address",
          });
          expect(notAllowed).toHaveProperty("details", {
            layer: "project_configuration",
            rule: "loopback",
            address: "127.0.0.1",
          });
        },
      );
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );
});
