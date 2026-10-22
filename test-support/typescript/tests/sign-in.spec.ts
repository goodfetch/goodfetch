// Seam B (spec #326): test support's Playwright `signIn` fixture, used as a
// served Console test uses it, on the `daemon` fixture's Daemon and the
// stand-in Console of `console-stand-in.ts` until area 12's is built. A
// person signs in through the executable's single-user link, uses a passkey
// through the virtual authenticator, and signs in through the pinned OIDC
// provider; neither the fixture nor a test writes a cookie, so the session
// cookie is the Console's own, `HttpOnly`.
import { expect, OIDC_PERSON, test } from "@goodfetch/test-support/playwright";
import { standInConsole } from "./console-stand-in.ts";

test.use({ consoleServer: standInConsole() });

test("signs in through the executable's single-user sign-in link", async ({
  context,
  daemon,
  page,
  signIn,
}) => {
  await signIn.withLink(page);

  await expect(page.locator("#status")).toHaveText(/^Signed in as /);
  expect(await context.cookies(daemon.url)).toEqual([
    expect.objectContaining({ httpOnly: true, sameSite: "Strict" }),
  ]);
});

test("registers a passkey and steps up with it through the virtual authenticator", async ({
  page,
  signIn,
}) => {
  await signIn.withLink(page);
  await page.getByRole("button", { name: "Register a passkey" }).click();
  await expect(page.getByRole("status")).toHaveText("Passkey registered");

  await page.getByRole("button", { name: "Confirm with your passkey" }).click();

  await expect(page.getByRole("status")).toHaveText("Confirmed with your passkey");
});

test("signs in through the OIDC provider", async ({ page, signIn }) => {
  await signIn.withOidc(page);

  await expect(page.locator("#status")).toHaveText(`Signed in as ${OIDC_PERSON.login}`);
});

test("fails when the OIDC provider refuses the person's password", async ({ page, signIn }) => {
  const signedIn = signIn.withOidc(page, { login: OIDC_PERSON.login, password: "wrong" });

  await expect(signedIn).rejects.toMatchObject({ code: "sign_in_failed" });
});

test("fails at once when the OIDC provider answers anything but its login page", async ({
  page,
  signIn,
  oidcProvider,
  failOnErrors,
}) => {
  test.setTimeout(30_000);
  await page.route(
    (url) => url.href.startsWith(oidcProvider.issuer),
    (route) =>
      route.request().method() === "POST"
        ? route.fulfill({ status: 400, contentType: "text/plain", body: "no such sign-in" })
        : route.fallback(),
  );

  const signedIn = signIn.withOidc(page);

  await expect(signedIn).rejects.toMatchObject({
    code: "sign_in_failed",
    message: expect.stringContaining("no such sign-in"),
  });
  // The provider's own page made the refused request on purpose, which the
  // browser also logs.
  const taken = await failOnErrors.take();
  expect(taken.map(({ kind }) => kind).toSorted()).toEqual(["console", "request_failed"]);
  // The routed plain-text answer is no page of the Console's for the axe pass.
  await page.close();
});

test.describe("on a Console whose sign-in link has expired", () => {
  test.use({ consoleServer: standInConsole({ expiredLinks: true }) });

  test("fails to sign in through the link", async ({ page, signIn }) => {
    const signedIn = signIn.withLink(page);

    await expect(signedIn).rejects.toMatchObject({ code: "sign_in_failed" });
  });
});
