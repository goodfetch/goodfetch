// A program's Sessions (ADRs 0092 and 0078), on an Attached engine that
// plays a Scripted engine's script (ADR 0069): `run.newSession` sends its
// `source` beside its `settings`, each Session's pages are its own, and
// closing one closes its pages. The script fails on a message it does not
// expect and on a step left unused, and the engine writes that failure to
// its stderr, which the logger gets at warning: an empty `warned` says the
// SDK sent exactly what the script holds.
import { ClosedError, Collie, ProgramError, SettingInvalidError } from "@goodfetch/collie";
import { failure, recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const SHOP_URL = "https://shop.test/";

describe("A Collie Run's Sessions", () => {
  test(
    "open from their source, keep their own pages, and close with them",
    { tags: ["client_case/sessions-open-from-their-source", "attached", "story:11-92"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/sessions-open-from-their-source.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start();
      const saved = await run.newSession({
        source: { kind: "saved_identity", name: "shop" },
        settings: { browser: { accessMode: "standard" } },
      });
      const savedPage = await saved.newPage();
      const fresh = await run.newSession({ source: null });
      await fresh.newPage();

      await saved.close();
      const afterClose = await Promise.all([
        failure(savedPage.goto(SHOP_URL)),
        failure(saved.newPage()),
      ]);
      await fresh.newPage();
      await run.newPage();
      const notRegistrable = await failure(
        run.newSession({
          source: { kind: "cdp_url", url: "http://127.0.0.1:9222", domains: ["www.example.com"] },
        }),
      );
      await run.finish();
      await collie.close();

      await clientCase.check(
        "the closed Session's page and the Session itself raise ClosedError without a message",
        () => {
          for (const refused of afterClose) {
            expect(refused).toBeInstanceOf(ClosedError);
          }
          expect(afterClose).toMatchObject([
            { code: "closed", group: "program" },
            { code: "closed", group: "program" },
          ]);
        },
      );
      await clientCase.check(
        "a domain that is no registrable one raises SettingInvalidError whose fix names the registrable domain",
        () => {
          expect(notRegistrable).toBeInstanceOf(SettingInvalidError);
          expect(notRegistrable).toBeInstanceOf(ProgramError);
          expect(notRegistrable).toMatchObject({
            code: "setting_invalid",
            group: "program",
            retryable: false,
            fix: "Give `example.com` in `domains`",
          });
          expect(notRegistrable).toHaveProperty("details", {
            key: "session.source",
            layer: "run",
            reason:
              "`www.example.com` in `domains` is no registrable domain, whose is `example.com`",
          });
        },
      );
      await clientCase.check(
        "each call sent its source, settings and Session as the script holds them, and the others opened pages still",
        () => expect(warned).toEqual([]),
      );
    },
  );

  test(
    "are reached from their pages without a call, the default one shared by the Run's pages",
    { tags: ["client_case/a-page-reaches-its-session", "attached", "story:11-92"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/a-page-reaches-its-session.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start();
      const first = await run.newPage();
      const second = await run.newPage();
      const own = await run.newSession({ source: null });
      const ownPage = await own.newPage();

      const defaultSession = first.session();
      await defaultSession.close();
      const afterClose = await failure(second.goto(SHOP_URL));
      const reopened = await failure(run.newPage());
      await ownPage.session().newPage();
      await run.finish();
      await collie.close();

      await clientCase.check(
        "the Run's two pages reach one default Session, and a page of the program's Session reaches that Session",
        () => {
          expect(second.session()).toBe(defaultSession);
          expect(ownPage.session()).toBe(own);
        },
      );
      await clientCase.check(
        "closing the default Session through a page closes the other page, and a later run.newPage() raises ClosedError",
        () => {
          expect(afterClose).toBeInstanceOf(ClosedError);
          expect(reopened).toBeInstanceOf(ClosedError);
          expect([afterClose, reopened]).toMatchObject([
            { code: "closed", group: "program" },
            { code: "closed", group: "program" },
          ]);
        },
      );
      await clientCase.check(
        "no call reached a page's Session but those the program made, as the script holds them",
        () => expect(warned).toEqual([]),
      );
    },
  );
});
