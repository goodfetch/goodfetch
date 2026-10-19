// Seam B (spec #326): the Playwright tools served tests rely on work under
// the production CSP's Trusted Types (ADR 0068), on every browser the
// project runs. Each tool's proof is its own test, against a fixture page
// that sets `require-trusted-types-for 'script'`; the first test shows the
// page enforces it, so a proof cannot pass on a browser that ignores it.
// The proofs run on test support's `test`, as served tests do, so a tool
// that breaks the CSP also fails `failOnErrors`.
import { AxeBuilder } from "@axe-core/playwright";
import { expect, test as base } from "@playwright/test";
import { test as served } from "@goodfetch/test-support/playwright";
import { cspPagesFixture, type CspPages } from "./csp-pages.ts";

const unwatched = base.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });
const test = served.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture });

unwatched.describe("the production CSP", () => {
  unwatched("refuses a string assigned to an HTML sink", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("trusted-types"));

    await expect(page.getByText(/^refused: /)).toHaveText("refused: TypeError");
  });
});

test.describe("under Trusted Types", () => {
  test("page.clock sets the page's time and fires its timers", async ({ page, cspPages }) => {
    await page.clock.install({ time: new Date("2026-01-05T08:59:00Z") });
    await page.clock.pauseAt(new Date("2026-01-05T09:00:00Z"));
    await page.goto(cspPages.url("clock"));
    await expect(page.locator("#now")).toHaveText("2026-01-05T09:00:00.000Z");

    await page.clock.runFor(60_000);

    await expect(page.locator("#timer")).toHaveText("fired at 2026-01-05T09:01:00.000Z");
  });

  test("routeWebSocket's shim forwards to the real server with connectToServer()", async ({
    page,
    cspPages,
  }) => {
    const forwarded: string[] = [];
    await page.routeWebSocket("**/echo", (socket) => {
      const server = socket.connectToServer();
      socket.onMessage((message) => {
        forwarded.push(String(message));
        server.send(message);
      });
    });

    await page.goto(cspPages.url("websocket"));

    await expect(page.locator("#received")).toHaveText("echo:ping");
    expect(forwarded).toEqual(["ping"]);
  });

  test("axe's injected script finds a violation", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("inaccessible"));

    const results = await new AxeBuilder({ page }).analyze();

    expect(results.violations.map((violation) => violation.id)).toContain("image-alt");
  });

  test("axe's injected script passes an accessible page", async ({ page, cspPages }) => {
    await page.goto(cspPages.url("accessible"));

    const results = await new AxeBuilder({ page }).analyze();

    expect(results.violations).toEqual([]);
  });

  test("the virtual authenticator registers a passkey and signs in with it", async ({
    context,
    page,
    cspPages,
  }) => {
    await context.credentials.install();
    await page.goto(cspPages.url("passkey"));
    await page.getByRole("button", { name: "Register" }).click();
    await expect(page.locator("#result")).toHaveText("registered public-key");

    await page.getByRole("button", { name: "Sign in" }).click();

    await expect(page.locator("#result")).toHaveText("signed in public-key");
  });
});
