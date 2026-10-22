// The local OIDC provider served tests sign in through (ADR 0068): panva's
// `oidc-provider`, an OpenID Certified implementation, at the exact version
// the package pins, its tarball pinned by the lockfile's SHA-512 digest and
// installed with the workspace before ADR 0043's deny. It runs in the test
// runner's process on port 0 of the loopback address, and knows one Console
// client and one person.
//
// Its own login form stands in for an identity provider's: it signs the
// person in only with their password, and grants the Console's scopes in
// the same step, so `signIn` drives one form, as a person would.
import { randomBytes } from "node:crypto";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { text } from "node:stream/consumers";
import { Provider, type Configuration } from "oidc-provider";

/** The person the provider knows, and the password their sign-in takes. */
export interface OidcPerson {
  /** Their login, the `sub` of their ID token and userinfo. */
  readonly login: string;
  readonly password: string;
  /** Their name, the `name` claim of the `profile` scope. */
  readonly name: string;
}

/** The one person of every test's OIDC provider. */
export const OIDC_PERSON: OidcPerson = {
  login: "person",
  password: "correct horse battery staple",
  name: "Test Person",
};

/** What a Console needs to sign people in through the provider. */
export interface OidcClient {
  /** The provider's issuer, `http://127.0.0.1:<port>`; its discovery document is at `/.well-known/openid-configuration`. */
  readonly issuer: string;
  readonly clientId: string;
  /** The client's secret, sent with `client_secret_basic`. */
  readonly clientSecret: string;
  /**
   * The path of the Console's redirect URI. As for a native client on the
   * loopback address (RFC 8252), any port of `http://127.0.0.1` is allowed,
   * so one provider serves every test's Console.
   */
  readonly redirectPath: string;
}

/** A started provider. */
export interface StartedOidcProvider {
  readonly client: OidcClient;
  close(): Promise<void>;
}

const CLIENT_ID = "goodfetch-console";
const REDIRECT_PATH = "/sso/callback";
const INTERACTION = /^\/interaction\/([\w-]+)$/;

/** Starts the provider on port 0 of the loopback address. */
export async function startOidcProvider(): Promise<StartedOidcProvider> {
  const clientSecret = randomBytes(32).toString("base64url");
  // The issuer names the port, which is known once the server listens.
  let provider: Provider | undefined;
  const server = createServer((request, response) => {
    if (provider === undefined) {
      response.writeHead(503).end();
      return;
    }
    void answer(provider, request, response);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("the OIDC provider listens on no TCP port");
  }
  const issuer = `http://127.0.0.1:${address.port}`;
  const configuration: Configuration = {
    clients: [
      {
        client_id: CLIENT_ID,
        client_secret: clientSecret,
        application_type: "native",
        redirect_uris: [`http://127.0.0.1${REDIRECT_PATH}`],
        grant_types: ["authorization_code"],
        response_types: ["code"],
        token_endpoint_auth_method: "client_secret_basic",
      },
    ],
    claims: { openid: ["sub"], profile: ["name"] },
    cookies: { keys: [randomBytes(32).toString("base64url")] },
    features: { devInteractions: { enabled: false } },
    interactions: { url: (_context, interaction) => `/interaction/${interaction.uid}` },
    findAccount: (_context, sub) =>
      sub === OIDC_PERSON.login
        ? {
            accountId: sub,
            claims: () => ({ sub, name: OIDC_PERSON.name }),
          }
        : undefined,
  };
  provider = new Provider(issuer, configuration);
  return {
    client: { issuer, clientId: CLIENT_ID, clientSecret, redirectPath: REDIRECT_PATH },
    close: async () => {
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) => {
        server.close((error) => (error === undefined ? resolve() : reject(error)));
      });
    },
  };
}

/** Serves the login form at the provider's interaction URL, and everything else through the provider. */
async function answer(
  provider: Provider,
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  const path = new URL(request.url ?? "/", "http://provider").pathname;
  if (INTERACTION.exec(path) === null) {
    void provider.callback()(request, response);
    return;
  }
  try {
    const interaction = await provider.interactionDetails(request, response);
    if (request.method !== "POST") {
      response
        .writeHead(200, { "Content-Type": "text/html; charset=utf-8" })
        .end(loginPage(path, false));
      return;
    }
    const form = new URLSearchParams(await text(request));
    if (form.get("login") !== OIDC_PERSON.login || form.get("password") !== OIDC_PERSON.password) {
      response
        .writeHead(200, { "Content-Type": "text/html; charset=utf-8" })
        .end(loginPage(path, true));
      return;
    }
    const grant = new provider.Grant({
      accountId: OIDC_PERSON.login,
      clientId: String(interaction.params["client_id"]),
    });
    grant.addOIDCScope(String(interaction.params["scope"]));
    await provider.interactionFinished(
      request,
      response,
      { login: { accountId: OIDC_PERSON.login }, consent: { grantId: await grant.save() } },
      { mergeWithLastSubmission: false },
    );
  } catch (error) {
    response
      .writeHead(400, { "Content-Type": "text/plain; charset=utf-8" })
      .end(`no such sign-in: ${String(error)}`);
  }
}

/** The provider's login form; `refused` after a wrong login or password. */
function loginPage(action: string, refused: boolean): string {
  const alert = refused ? '<p role="alert">Wrong login or password.</p>' : "";
  return `<!doctype html>
<html lang="en">
  <head><meta charset="utf-8" /><title>Sign in to the test OIDC provider</title></head>
  <body>
    <main>
      <h1>Sign in</h1>
      ${alert}
      <form method="post" action="${action}">
        <label>Login <input name="login" autocomplete="username" /></label>
        <label>Password <input name="password" type="password" autocomplete="current-password" /></label>
        <button type="submit">Sign in</button>
      </form>
    </main>
  </body>
</html>`;
}
