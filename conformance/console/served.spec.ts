// What the Console needs from its target as a site (ADRs 0024, 0031, 0067
// and 0068): the single-user sign-in link signs the page in with an
// `HttpOnly` cookie and a request with none is refused; every route answers
// with the Console's security headers; and the page, walked across its
// screens, sends its target only catalog operations, at the one origin it
// was served from, which is its configuration (story 12-137).
import type { Page } from "@playwright/test";
import { expect, test } from "@goodfetch/test-support/conformance";
import { operationClasses } from "@goodfetch/console/generated/operations";
import { PRODUCT_NAME, startRun } from "./target.ts";

test(
  "the-single-user-link-signs-in-and-no-cookie-is-refused",
  {
    annotation: [
      { type: "capability", description: "sign-in" },
      { type: "covers", description: "story:12-1" },
      { type: "covers", description: "story:12-109" },
    ],
  },
  async ({ page, context, daemon, signIn }) => {
    await signIn.withLink(page);

    await expect(page.getByRole("main").getByRole("status")).toHaveText(
      new RegExp(`^Connected to ${PRODUCT_NAME[daemon.product]} \\d+\\.\\d+\\.\\d+\\S*\\.$`),
    );
    expect(new URL(page.url()).hash).toBe("");
    const cookies = await context.cookies(daemon.url);
    expect(cookies).toEqual([
      expect.objectContaining({ httpOnly: true, sameSite: "Strict", path: "/" }),
    ]);
    expect(cookies[0]?.value).not.toBe(daemon.token);
    // Outside the browser, so the request carries no cookie of the page's.
    expect((await fetch(daemon.session)).status).toBe(401);
  },
);

/** The headers every answer of the Console carries, whatever its status. */
const SECURITY_HEADERS = {
  "referrer-policy": "no-referrer",
  "cross-origin-opener-policy": "same-origin",
  "cross-origin-resource-policy": "same-origin",
  "x-content-type-options": "nosniff",
};

/** The directives of the Console's Content Security Policy every answer carries. */
const POLICY = [
  "default-src 'none'",
  "script-src 'self'",
  "connect-src 'self'",
  "frame-ancestors 'none'",
  "require-trusted-types-for 'script'",
  "trusted-types 'none'",
];

test(
  "every-route-carries-the-console-security-headers",
  {
    annotation: [
      { type: "capability", description: "security-headers" },
      { type: "covers", description: "story:12-128" },
      { type: "covers", description: "story:12-129" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await signIn.withLink(page);
    const assets: string[] = [];
    page.on("response", (response) => {
      const url = new URL(response.url());
      if (url.origin === new URL(daemon.url).origin && /\.(js|css|woff2)$/.test(url.pathname)) {
        assets.push(url.pathname);
      }
    });
    await page.goto(new URL("/runs", daemon.url).href);
    await expect(page.getByRole("main")).toBeVisible();
    expect(assets.length).toBeGreaterThan(0);
    const paths = [
      "/",
      "/runs",
      "/runs/a-run",
      "/needs-you",
      "/settings",
      "/session",
      "/protocol",
      "/protocol/events",
      "/a-route-the-console-does-not-have",
      ...new Set(assets),
    ];

    for (const path of paths) {
      // Outside the browser, so each answer is the target's own, signed in or not.
      const answer = await fetch(new URL(path, daemon.url));
      const policy = answer.headers.get("content-security-policy") ?? "";
      for (const directive of POLICY) {
        expect(policy, `${path}: ${directive}`).toContain(directive);
      }
      for (const [name, value] of Object.entries(SECURITY_HEADERS)) {
        expect(answer.headers.get(name), `${path}: ${name}`).toBe(value);
      }
    }
  },
);

/** What the page may send beside catalog operations: `hello` and the protocol's own notifications. */
function inCatalog(method: string): boolean {
  return method === "hello" || method.startsWith("$/") || method in operationClasses;
}

/** The method of a message the page sent, if it names one. */
function methodOf(payload: string | Buffer): string | undefined {
  if (typeof payload !== "string") {
    return undefined;
  }
  const message: unknown = JSON.parse(payload);
  return typeof message === "object" && message !== null && "method" in message
    ? String(message.method)
    : undefined;
}

/** Visits each screen both Products' builds carry, as a person moving through it. */
async function walk(page: Page, url: string): Promise<void> {
  for (const [path, shown] of [
    ["/runs", page.getByRole("table")],
    ["/runs/walked", page.getByText("walked").first()],
    ["/needs-you", page.getByText("Nothing needs you now.")],
    ["/settings", page.getByRole("button", { name: "Explain" })],
  ] as const) {
    await page.goto(new URL(path, url).href);
    await expect(shown).toBeVisible();
  }
}

test(
  "the-console-sends-only-catalog-operations-to-its-configured-origin",
  {
    annotation: [
      { type: "capability", description: "catalog" },
      { type: "covers", description: "story:12-137" },
    ],
  },
  async ({ page, context, daemon, signIn }) => {
    const origin = new URL(daemon.url).origin;
    const requests: string[] = [];
    const sent: string[] = [];
    context.on("request", (request) => requests.push(request.url()));
    page.on("websocket", (socket) => {
      requests.push(socket.url());
      socket.on("framesent", ({ payload }) => {
        const method = methodOf(payload);
        if (method !== undefined) {
          sent.push(method);
        }
      });
    });
    await startRun(daemon, "walked");

    await signIn.withLink(page);
    await walk(page, daemon.url);

    expect(sent).toEqual(expect.arrayContaining(["hello", "run.list", "events.subscribe"]));
    expect(sent.filter((method) => !inCatalog(method))).toEqual([]);
    const elsewhere = requests.filter((url) => {
      const { protocol, host } = new URL(url);
      return !["data:", "blob:"].includes(protocol) && `http://${host}` !== origin;
    });
    expect(elsewhere).toEqual([]);
  },
);
