// Seam B (spec #326): the served fixtures fail a test for the reason they
// name, by its Error code, where a test's own `test.fail` would pass on any
// failure: `context` and `request` refuse `bypassCSP` and a `storageState`
// (ADR 0068), `failOnErrors` fails a test whose page made an error, and
// `axe` one whose page has a violation (ADR 0073). Playwright runs the
// repository's configuration on a spec file of such tests through
// `repository-playwright.ts`, and each test's failure is read from its JSON
// report.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";
import { runRepositoryPlaywright } from "./repository-playwright.ts";

const FIXTURES = pathToFileURL(join(import.meta.dirname, "..", "src", "playwright.ts")).href;
const CSP_PAGES = pathToFileURL(join(import.meta.dirname, "csp-pages.ts")).href;

const SPEC = `import { expect, test as served } from ${JSON.stringify(FIXTURES)};
import { cspPagesFixture } from ${JSON.stringify(CSP_PAGES)};

const test = served.extend({ cspPages: cspPagesFixture });

const SIGNED_IN = {
  cookies: [{ name: "goodfetch_session", value: "forged", domain: "127.0.0.1", path: "/",
    expires: -1, httpOnly: true, secure: false, sameSite: "Strict" }],
  origins: [],
};

test.describe("context with bypassCSP", () => {
  test.use({ bypassCSP: true });
  test("context with bypassCSP", async ({ page }) => { await page.goto("about:blank"); });
});
test.describe("contextOptions with bypassCSP", () => {
  test.use({ contextOptions: { bypassCSP: true } });
  test("contextOptions with bypassCSP", async ({ page }) => { await page.goto("about:blank"); });
});
test.describe("request with bypassCSP", () => {
  test.use({ bypassCSP: true });
  test("request with bypassCSP", async ({ request }) => { await request.dispose(); });
});
test.describe("context with a storage state", () => {
  test.use({ storageState: SIGNED_IN });
  test("context with a storage state", async ({ page }) => { await page.goto("about:blank"); });
});
test.describe("contextOptions with a storage state", () => {
  test.use({ contextOptions: { storageState: SIGNED_IN } });
  test("contextOptions with a storage state", async ({ page }) => { await page.goto("about:blank"); });
});
test.describe("request with a storage state", () => {
  test.use({ storageState: SIGNED_IN });
  test("request with a storage state", async ({ request }) => { await request.dispose(); });
});

for (const name of ["inline-script", "console-error", "page-error", "missing-script", "broken-request"]) {
  test(\`page \${name}\`, async ({ page, cspPages }) => {
    await page.goto(cspPages.url(name));
    await expect(page.locator("main")).toBeVisible();
  });
}

test("page inaccessible", async ({ page, cspPages }) => {
  await page.goto(cspPages.url("inaccessible"));
  await expect(page.getByRole("heading", { name: "Inaccessible" })).toBeVisible();
});
`;

/** Each test's title, status and errors' messages, from Playwright's JSON report. */
function outcomes(report: string): Map<string, { status: string; error: string }> {
  const found = new Map<string, { status: string; error: string }>();
  const visit = (suite: Suite): void => {
    for (const spec of suite.specs ?? []) {
      const result = spec.tests[0]?.results[0];
      found.set(spec.title, {
        status: result?.status ?? "none",
        error: (result?.errors ?? []).map(({ message }) => message ?? "").join("\n"),
      });
    }
    for (const child of suite.suites ?? []) {
      visit(child);
    }
  };
  const parsed: { suites: Suite[] } = JSON.parse(readFileSync(report, "utf8"));
  parsed.suites.forEach(visit);
  return found;
}

interface Suite {
  readonly specs?: readonly {
    readonly title: string;
    readonly tests: readonly {
      readonly results: readonly {
        readonly status: string;
        readonly errors?: readonly { readonly message?: string }[];
      }[];
    }[];
  }[];
  readonly suites?: readonly Suite[];
}

describe("the served fixtures", () => {
  test("fail each test for its own reason, by its Error code", async ({ processes }) => {
    const { report } = await runRepositoryPlaywright(processes, {
      folder: "fixture-failures-test",
      spec: { name: "fixture-failures.spec.ts", text: SPEC },
      json: true,
    });

    const found = outcomes(report);
    const codes = Object.fromEntries(
      [...found].map(([title, { status, error }]) => [
        title,
        `${status} ${/TestSupportError\((\w+)\)/.exec(error)?.[1] ?? "no code"}`,
      ]),
    );
    expect(codes).toEqual({
      "context with bypassCSP": "failed bypass_csp_refused",
      "contextOptions with bypassCSP": "failed bypass_csp_refused",
      "request with bypassCSP": "failed bypass_csp_refused",
      "context with a storage state": "failed storage_state_refused",
      "contextOptions with a storage state": "failed storage_state_refused",
      "request with a storage state": "failed storage_state_refused",
      "page inline-script": "failed page_errors",
      "page console-error": "failed page_errors",
      "page page-error": "failed page_errors",
      "page missing-script": "failed page_errors",
      "page broken-request": "failed page_errors",
      "page inaccessible": "failed axe_violations",
    });
  });
});
