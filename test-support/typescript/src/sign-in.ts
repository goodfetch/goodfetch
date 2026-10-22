// Signing a served test in the way a person signs in (ADRs 0031 and 0068):
// the single-user sign-in link the executable gives, Playwright's virtual
// authenticator for passkeys and step-up, and the test's local OIDC
// provider for single sign-on. Nothing here writes a cookie or storage
// state: the session is whatever the Console sets on the way, and signing in
// ends once the Console answers its session address as signed in.
import type { BrowserContext, Page } from "@playwright/test";
import { TestSupportError } from "./errors.ts";
import { OIDC_PERSON, type OidcClient, type OidcPerson } from "./oidc-provider.ts";
import type { ServedDaemon } from "./served-daemon.ts";

/** How a served test signs in to its Console. */
export interface SignIn {
  /** Follows the single-user sign-in link the executable gives, in `page`. */
  withLink(page: Page): Promise<void>;
  /**
   * Starts the Console's single sign-on in `page` and signs `person` in on
   * the OIDC provider's login form; `OIDC_PERSON` by default.
   */
  withOidc(page: Page, person?: Pick<OidcPerson, "login" | "password">): Promise<void>;
}

/**
 * Installs the virtual authenticator on the test's context, for the
 * Console's passkeys and step-up, and gives the ways to sign in.
 */
export async function signInTo(
  context: BrowserContext,
  daemon: ServedDaemon,
  oidc: OidcClient,
): Promise<SignIn> {
  await context.credentials.install();
  return {
    withLink: async (page) => {
      const link = await daemon.signInLink();
      await page.goto(link);
      // A link that carries its token in the fragment is signed in once
      // the page has swapped it and taken it out of the address.
      await page.waitForURL((url) => url.hash === "");
      await expectSignedIn(page, daemon, "the Console did not sign in through its link");
    },
    withOidc: async (page, person = OIDC_PERSON) => {
      await page.goto(daemon.singleSignOn);
      const form = page.getByRole("form").or(page.locator("form"));
      await form.getByLabel("Login").fill(person.login);
      await form.getByLabel("Password").fill(person.password);
      const submitted = page.waitForResponse(
        (response) =>
          response.request().method() === "POST" && response.url().startsWith(oidc.issuer),
      );
      await form.getByRole("button", { name: "Sign in" }).click();
      const answer = await submitted;
      if (answer.status() !== 303) {
        // Only the login page refusing a password has an alert: read it
        // without waiting, else the answer itself, so no other answer waits
        // out the test's timeout.
        const alerts = page.getByRole("alert");
        const why =
          (await alerts.count()) > 0
            ? await alerts.first().textContent()
            : `HTTP ${answer.status()}, ${await answer.text()}`;
        throw new TestSupportError(
          "sign_in_failed",
          `the OIDC provider did not sign ${person.login} in: ${why ?? `HTTP ${answer.status()}`}`,
        );
      }
      await page.waitForURL((url) => url.href.startsWith(daemon.url));
      await expectSignedIn(
        page,
        daemon,
        `the Console did not sign ${person.login} in from the OIDC provider`,
      );
    },
  };
}

/** Fails with `sign_in_failed` unless the Console answers its session address as signed in. */
async function expectSignedIn(page: Page, daemon: ServedDaemon, failure: string): Promise<void> {
  const session = await page.request.get(daemon.session, { maxRedirects: 0 });
  if (!session.ok()) {
    throw new TestSupportError(
      "sign_in_failed",
      `${failure}: ${daemon.session} answered ${session.status()}`,
    );
  }
}
