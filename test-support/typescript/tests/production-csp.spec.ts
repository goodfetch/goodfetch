// The production CSP the fixture pages and the stand-in Console serve
// (`csp-pages.ts`) is the one the executable's own Console serves (ADR 0031),
// read from the commit's Collie executable through the default
// `executableConsole`, so the two cannot drift apart unseen.
import { expect, test } from "@goodfetch/test-support/playwright";
import { PRODUCTION_CSP } from "./csp-pages.ts";

test("is the Content Security Policy the executable's Console serves", async ({
  daemon,
  request,
}) => {
  const answer = await request.get(daemon.url);

  expect(answer.headers()["content-security-policy"]).toBe(PRODUCTION_CSP);
});
