// A program's pages on an Attached engine of Collie's executable of the same
// commit (ADR 0069), which launches the browser of the test's own Browser
// cache and reaches the Proving ground's pages through the test's home
// (`withBrowser`). The form program is the one the unit tests run on the
// Scripted engine. `goto`, `goBack`, `goForward` and `request` answer a
// `Response`, whose body reads, decodes and fails as ADRs 0085, 0118 and 0119
// say; Rust's `tests/it/sdk/page.rs` and Python's `test_page_attached.py` are
// their twins.
import { Collie } from "@goodfetch/collie";
import {
  failure,
  goodfetchFailure,
  judgeTask,
  provingGroundObservations,
  provingGroundUrl,
  taskStartUrl,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { fillTheForm } from "./programs/form.ts";

describe("a program's pages on an Attached engine", () => {
  test.override("provingGroundSeed", 2400);
  test.override("withBrowser", true);

  test(
    "runs the form program on the Proving ground's form page and reads its Markdown",
    { tags: ["client_case/page-form-program", "attached", "story:11-77"] },
    async ({ env, provingGround, clientCase }) => {
      const form = await taskStartUrl(provingGround, "actionability/20");
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
      const missing = new URL("/errors/404", await taskStartUrl(provingGround, "statuses/1")).href;
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const navigated = await page.goto(missing);

      await clientCase.check("a 404 is a value: the document's Response", () =>
        expect([navigated?.status(), navigated?.url(), navigated?.ok()]).toEqual([
          404,
          missing,
          false,
        ]),
      );
    },
  );

  test(
    "opens a second page of a Run in the same Session, and a new Session's page in another",
    { tags: ["client_case/second-new-page-in-the-default-session", "attached", "story:11-77"] },
    async ({ env, provingGround, clientCase }) => {
      const form = await taskStartUrl(provingGround, "actionability/20");
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
        expect(shared).toBe("by the first page"),
      );
      await clientCase.check("a page of another Session reads nothing of it", () =>
        expect(apart).toBeNull(),
      );
    },
  );

  test(
    "reads a product page's fields in one call, typed by their declaration",
    { tags: ["client_case/page-read-fields", "attached", "story:11-77", "operation:page.read"] },
    async ({ env, provingGround, clientCase }) => {
      const product = await taskStartUrl(provingGround, "catalog/2");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();
      await page.goto(product);

      const read = await page.read({
        id: { locator: { kind: "css", selector: "span.id" } },
        name: { locator: { kind: "css", selector: "h1.name" } },
        price: {
          locator: { kind: "css", selector: "span.price" },
          attribute: "data-price",
          transform: { kind: "number", locale: "en" },
        },
        missing: { locator: { kind: "css", selector: ".no-such-element" } },
      });
      const { missing, ...record } = read.values;
      const answer = await judgeTask(provingGround, { task: "catalog/2", records: [record] });
      clientCase.oracleAnswer(answer);

      await clientCase.check("the oracle verifies the Record the fields answer", () =>
        expect(answer.outcome).toBe("verified"),
      );
      await clientCase.check("the number field answers a number", () =>
        expect(typeof record.price).toBe("number"),
      );
      await clientCase.check("a field with no element is null, with its reason", () => {
        expect(missing).toBeNull();
        expect(read.unread).toEqual([{ field: "/missing", reason: "no_element" }]);
      });
    },
  );

  test(
    "refuses a call on a closed page, and on a Locator built on it, with closed",
    { tags: ["client_case/closed-page-raises-closed", "attached", "error:closed"] },
    async ({ env, provingGround, clientCase }) => {
      const form = await taskStartUrl(provingGround, "actionability/20");
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

/** The Proving ground's host of edge cases of the HTTP layer. */
const FETCH_EDGE = "fetch-edge.test";

describe("a page's responses on an Attached engine", () => {
  test.override("provingGroundSeed", 3394);
  test.override("withBrowser", true);

  test(
    "reads a navigation's head and its document's text",
    { tags: ["attached", "story:11-84", "story:11-85", "operation:response.body"] },
    async ({ env, provingGround }) => {
      const product = await taskStartUrl(provingGround, "catalog/2");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const response = await page.goto(product);
      const [text, body] = await Promise.all([response?.text(), response?.body()]);

      expect([response?.status(), response?.url(), response?.ok()]).toEqual([200, product, true]);
      expect(response?.headers()["content-type"]).toMatch(/^text\/html/);
      expect(response?.encoding()).toBe("UTF-8");
      expect(text).toContain('<h1 class="name">');
      expect(text).toBe(new TextDecoder().decode(body));
    },
  );

  // A program's own `goto` is outside Politeness (ADR 0086): the redirect's
  // `Retry-After` asks for an hour, far past `timeout.navigation`, so its
  // target answering is the proof, with no measured time (ADR 0118). A file
  // the browser downloads answers no response and the page keeps its
  // document, which a reload loads again (ADR 0119).
  test(
    "follows a redirect's Retry-After at once and answers null for a file",
    {
      tags: [
        "client_case/goto-skips-a-redirects-retry-after-and-answers-none-for-a-file",
        "attached",
        "story:11-84",
        "operation:page.goto",
      ],
    },
    async ({ env, provingGround, clientCase }) => {
      const redirect = provingGroundUrl(provingGround, FETCH_EDGE, "/retry-after/302/seconds/long");
      const target = provingGroundUrl(provingGround, FETCH_EDGE, "/target");
      const document = await taskStartUrl(provingGround, "catalog/2");
      const file = new URL("/files/report.pdf", await taskStartUrl(provingGround, "files/2")).href;
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const followed = await page.goto(redirect);
      await page.goto(document);
      const downloaded = await page.goto(file);
      const reloaded = await page.reload();

      await clientCase.check("a 302 whose Retry-After asks for an hour is followed at once", () =>
        expect([followed?.status(), followed?.url()]).toEqual([200, target]),
      );
      await clientCase.check("a goto to a file the browser downloads answers null", () =>
        expect(downloaded).toBeNull(),
      );
      await clientCase.check("the page keeps its document, which a reload loads again", () =>
        expect([reloaded?.status(), reloaded?.url()]).toEqual([200, document]),
      );
    },
  );

  test(
    "decodes text with a legacy charset and JSON past a BOM, and names the URL of no JSON",
    {
      tags: [
        "attached",
        "story:11-85",
        "operation:page.request",
        "operation:response.body",
        "error:response_not_json",
      ],
    },
    async ({ env, provingGround }) => {
      const shiftJis = provingGroundUrl(provingGround, FETCH_EDGE, "/text/shift-jis");
      const bom = provingGroundUrl(provingGround, FETCH_EDGE, "/json/bom");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const textResponse = await page.request(shiftJis);
      const japanese = await textResponse.text();
      const notJson = await failure(textResponse.json());
      const parsed = await (await page.request(bom)).json();

      expect([japanese, parsed]).toEqual(["日本語", { bom: true }]);
      expect(notJson).toMatchObject({ code: "response_not_json", url: shiftJis });
    },
  );

  test(
    "raises response_truncated for a cut body from body, text and json",
    { tags: ["attached", "story:11-85", "operation:response.body", "error:response_truncated"] },
    async ({ env, provingGround }) => {
      const cut = provingGroundUrl(provingGround, FETCH_EDGE, "/cut/content-length");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const response = await page.request(cut);
      const failures = [
        await goodfetchFailure(response.body()),
        await goodfetchFailure(response.text()),
        await goodfetchFailure(response.json()),
      ];

      expect(failures.map((failed) => failed.code)).toEqual(
        Array.from({ length: 3 }, () => "response_truncated"),
      );
    },
  );

  test(
    "raises body_unavailable naming the waiter for a body the browser dropped",
    { tags: ["attached", "story:11-85", "operation:response.body", "error:body_unavailable"] },
    async ({ env, provingGround }) => {
      const first = await taskStartUrl(provingGround, "catalog/2");
      const then = await taskStartUrl(provingGround, "actionability/20");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const left = await page.goto(first);
      if (left === null) {
        throw new Error("a document has a response");
      }
      await page.goto(then);
      const dropped = await failure(left.text());

      expect(dropped).toMatchObject({
        code: "body_unavailable",
        fix: expect.stringContaining("page.waitForRequestFinished"),
      });
    },
  );

  test(
    "answers the document's Response going back and forward, and null past the history",
    { tags: ["attached", "story:11-84", "operation:page.go_back", "operation:page.go_forward"] },
    async ({ env, provingGround }) => {
      const first = await taskStartUrl(provingGround, "catalog/2");
      const then = await taskStartUrl(provingGround, "actionability/20");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();
      await page.goto(first);
      await page.goto(then);

      const back = await page.goBack();
      const forward = await page.goForward();
      const past = await page.goForward();

      expect([back?.url(), forward?.url(), past]).toEqual([first, then, null]);
    },
  );
});

describe("page.request through the page's Session on an Attached engine", () => {
  test.override("provingGroundSeed", 1303);
  test.override("withBrowser", true);

  test(
    "sends the cookie a page of its Session got, through the Run's proxy",
    {
      tags: [
        "client_case/page-request-sends-the-sessions-cookie-through-its-proxy",
        "attached",
        "story:11-87",
        "operation:page.request",
      ],
    },
    async ({ env, provingGround, clientCase }) => {
      const openProxy = provingGround.fixtures.proxies["open"];
      const storage = provingGroundUrl(provingGround, "scope.test", "/storage?mark=page-request");
      const about = provingGroundUrl(provingGround, "scope.test", "/about?by=page-request");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start({ settings: { proxy: { server: `http://${openProxy}` } } });
      const page = await run.newPage();
      await page.goto(storage);

      const response = await page.request(about);
      await run.finish();
      const observations = await provingGroundObservations(provingGround);
      const sent = observations.requests
        .filter(({ host, target }) => host === "scope.test" && target === "/about?by=page-request")
        .map(({ cookies }) => cookies);

      await clientCase.check("`page.request` from the Session answers 200", () =>
        expect(response.status()).toBe(200),
      );
      await clientCase.check("it sends the cookie a page of its Session got", () =>
        expect(sent).toEqual([[["mark", "page-request"]]]),
      );
      await clientCase.check("it leaves through the Run's proxy", () =>
        expect(observations).toMatchObject({
          relays: expect.arrayContaining([
            expect.objectContaining({
              proxy: "open",
              outcome: "relayed",
              target: expect.stringMatching(/\/about\?by=page-request$/),
            }),
          ]),
        }),
      );
    },
  );

  test(
    "follows its maxRedirects, answering the 3xx at 0, and refuses a count above 19",
    {
      tags: [
        "client_case/page-request-follows-its-max-redirects-and-refuses-a-count-above-19",
        "attached",
        "story:11-87",
        "operation:page.request",
        "error:too_many_redirects",
        "error:setting_invalid",
      ],
    },
    async ({ env, provingGround, clientCase }) => {
      const chain = provingGroundUrl(provingGround, FETCH_EDGE, "/redirect/hops/5");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();

      const refused = await goodfetchFailure(page.request(chain, { maxRedirects: 20 }));
      const unfollowed = await page.request(chain, { maxRedirects: 0 });
      const within = await page.request(chain, { maxRedirects: 5 });
      const past = await goodfetchFailure(page.request(chain, { maxRedirects: 3 }));

      await clientCase.check("`maxRedirects` 20 is refused, naming the option", () =>
        expect(refused).toMatchObject({
          code: "setting_invalid",
          details: { key: "max_redirects" },
        }),
      );
      await clientCase.check("`maxRedirects` 0 answers the 3xx itself", () => {
        expect(unfollowed.status()).toBe(302);
        expect(unfollowed.url()).toMatch(/\/redirect\/hops\/5$/);
        expect(unfollowed.headers()["location"]).toBe("/redirect/hops/4");
      });
      await clientCase.check("a chain within its count answers its last response", () => {
        expect(within.status()).toBe(200);
        expect(within.url()).toMatch(/\/redirect\/hops\/0$/);
      });
      await clientCase.check(
        "past its count, `too_many_redirects` names the chain, its hops and the option",
        () =>
          expect(past).toMatchObject({
            code: "too_many_redirects",
            url: chain,
            details: {
              hops: [expect.any(String), expect.any(String), expect.any(String)],
              limit: { kind: "option", name: "max_redirects", count: 3 },
            },
          }),
      );
    },
  );
});
