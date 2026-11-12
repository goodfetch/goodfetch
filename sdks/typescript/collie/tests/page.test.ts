// A program's pages on an Attached engine of Collie's executable of the same
// commit (ADR 0069), which launches the browser of the test's own Browser
// cache and reaches the Proving ground's pages through the test's home
// (`withBrowser`). The form program is the one the unit tests run on the
// Scripted engine.
import { Collie } from "@goodfetch/collie";
import {
  failure,
  provingGroundRoutes,
  test,
  type ProvingGroundDescription,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { fillTheForm } from "./programs/form.ts";

/** The URL the Proving ground's task `id` starts at. */
async function startUrl(provingGround: ProvingGroundDescription, id: string): Promise<string> {
  const answer = await fetch(`http://${provingGround.address}${provingGroundRoutes().tasks}`);
  const list: unknown = await answer.json();
  if (typeof list === "object" && list !== null && "tasks" in list && Array.isArray(list.tasks)) {
    const tasks: unknown[] = list.tasks;
    for (const task of tasks) {
      if (
        typeof task === "object" &&
        task !== null &&
        "id" in task &&
        task.id === id &&
        "start_url" in task &&
        typeof task.start_url === "string"
      ) {
        return task.start_url;
      }
    }
  }
  throw new TypeError(`the Proving ground's task list has no ${id}`);
}

describe("a program's pages on an Attached engine", () => {
  test.override("provingGroundSeed", 2400);
  test.override("withBrowser", true);

  test(
    "runs the form program on the Proving ground's form page and reads its Markdown",
    { tags: ["client_case/page-form-program", "attached", "story:11-77"] },
    async ({ env, provingGround, clientCase }) => {
      const form = await startUrl(provingGround, "actionability/20");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });

      const filled = await fillTheForm(collie, form);

      await clientCase.check("goto answers the form page's document", () =>
        expect(filled.status).toBe(200),
      );
      await clientCase.check("the page reached its end state: the form was sent", () =>
        expect(filled.markdown).toContain("done"),
      );
      await clientCase.check("the Run finished", () => expect(filled.lifecycle).toBe("finished"));
    },
  );

  test(
    "returns the Response of a 404 from goto",
    { tags: ["client_case/goto-404-returns-the-response", "attached", "story:11-84"] },
    async ({ env, provingGround, clientCase }) => {
      const missing = new URL("/errors/404", await startUrl(provingGround, "statuses/1")).href;
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const navigated = await page.goto(missing);

      await clientCase.check("a 404 is a value: the document's Response", () =>
        expect(navigated.response).toMatchObject({ status: 404, url: missing }),
      );
    },
  );

  test(
    "opens a second page of a Run in the same Session, and a new Session's page in another",
    { tags: ["client_case/second-new-page-in-the-default-session", "attached", "story:11-77"] },
    async ({ env, provingGround, clientCase }) => {
      const form = await startUrl(provingGround, "actionability/20");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const first = await run.newPage();
      const second = await run.newPage();
      const session = await run.newSession();
      const other = await session.newPage();
      await Promise.all([first.goto(form), second.goto(form), other.goto(form)]);

      await first.evaluate("localStorage.setItem('kept', 'by the first page')");
      const shared = await second.evaluate("localStorage.getItem('kept')");
      const apart = await other.evaluate("localStorage.getItem('kept')");

      await clientCase.check("the second page reads what the first stored: one Session", () =>
        expect(shared.value).toBe("by the first page"),
      );
      await clientCase.check("a page of another Session reads nothing of it", () =>
        expect(apart.value).toBeNull(),
      );
    },
  );

  test(
    "refuses a call on a closed page, and on a Locator built on it, with closed",
    { tags: ["client_case/closed-page-raises-closed", "attached", "error:closed"] },
    async ({ env, provingGround, clientCase }) => {
      const form = await startUrl(provingGround, "actionability/20");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();
      await page.goto(form);

      const button = page.locator("#submit");
      await page.close();

      const [navigation, click] = await Promise.all([
        failure(page.goto(form)),
        failure(button.click()),
      ]);

      await clientCase.check("goto on the closed page fails with closed", () =>
        expect(navigation).toMatchObject({ code: "closed", group: "program" }),
      );
      await clientCase.check("a locator of the closed page fails with closed", () =>
        expect(click).toMatchObject({ code: "closed", group: "program" }),
      );
    },
  );
});
