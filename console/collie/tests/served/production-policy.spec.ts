// Collie's Console as its executable serves it (ADRs 0031, 0057 and 0068):
// signed in through its link, the page opens on the shell named "Goodfetch
// Collie", and the production Content Security Policy refuses an inline
// script injected into the page and a string sent to a script sink. The
// tools served tests rely on are proved under the same policy's Trusted
// Types on Chromium, Firefox and WebKit by test support's
// `trusted-types.spec.ts` (#372).
import { expect, test } from "@goodfetch/test-support/playwright";

test(
  "the Console opens on its shell, named Goodfetch Collie, then Collie",
  { annotation: { type: "covers", description: "story:12-128" } },
  async ({ page, signIn }) => {
    await signIn.withLink(page);

    await expect(page).toHaveTitle("Goodfetch Collie");
    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Goodfetch Collie");
    await expect(page.getByText("Collie's Console.", { exact: false })).toBeVisible();
  },
);

test(
  "the production policy refuses an injected inline script and a string sent to a script sink",
  { annotation: { type: "covers", description: "story:12-128" } },
  async ({ page, daemon, signIn, failOnErrors }) => {
    await signIn.withLink(page);
    // A page that carries an inline script, as an injection into the served
    // HTML would, still under the policy the executable sent with it.
    await page.route(daemon.url, async (route) => {
      const response = await route.fetch();
      const html = await response.text();
      await route.fulfill({
        response,
        body: html.replace("</head>", "<script>document.title = 'injected';</script></head>"),
      });
    });
    await page.goto(daemon.url);
    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Goodfetch Collie");

    const sink = await page.evaluate(() => {
      try {
        document.createElement("script").text = "document.title = 'injected';";
        return "accepted";
      } catch (error) {
        return error instanceof Error ? error.name : String(error);
      }
    });

    expect(sink).toBe("TypeError");
    await expect(page).toHaveTitle("Goodfetch Collie");
    await expect
      .poll(() => failOnErrors.errors.map((error) => error.kind))
      .toEqual(expect.arrayContaining(["csp_violation", "csp_report"]));
    // A page Playwright fulfilled has no address the browser knows, so its
    // Console may not open its WebSocket to the Daemon on loopback (Local
    // Network Access): the page closes before its errors are taken.
    await page.close();
    const refused = await failOnErrors.take();
    expect(
      refused
        .filter((error) => error.kind === "csp_violation")
        .map((error) => error.text.split(" ")[0]),
    ).toEqual(expect.arrayContaining(["script-src-elem", "require-trusted-types-for"]));
  },
);
