// The production CSP the fixture pages and the stand-in Console serve
// (`csp-pages.ts`) is the one the executable's own Console serves (ADR 0031),
// read from the commit's Collie executable through the default
// `executableConsole`, so the two cannot drift apart unseen. The executable's
// Console frames its content origin, the only source its policy adds, so the
// test reads that origin off the policy, checks it is the Daemon's content
// origin, a second origin on the Console's host, and holds the whole policy to
// the exact text the fixtures' policy takes with it.
import { expect, test } from "@goodfetch/test-support/playwright";
import { productionCsp } from "./csp-pages.ts";

test("is the Content Security Policy the executable's Console serves", async ({
  daemon,
  request,
}) => {
  const answer = await request.get(daemon.url);
  const policy = answer.headers()["content-security-policy"];

  // The content origin is a bare `http://127.0.0.1:<port>`, no wildcard or path,
  // and not the Console's own origin, which a frame must not share.
  const framed = /(?:^|; )frame-src ([^;]*)(?:;|$)/.exec(policy ?? "")?.[1];
  expect(framed).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
  expect(framed).not.toBe(new URL(daemon.url).origin);
  expect(policy).toBe(productionCsp(framed));

  // That origin is the content origin: its own policy lets this Console frame it.
  const content = await request.get(`${framed}/`);
  const contentPolicy = content.headers()["content-security-policy"];
  const ancestors = contentPolicy
    ?.split("; ")
    .find((directive) => directive.startsWith("frame-ancestors "))
    ?.split(" ")
    .slice(1);
  expect(ancestors).toContain(new URL(daemon.url).origin);
});
