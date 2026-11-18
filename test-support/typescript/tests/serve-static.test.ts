// Seam B (spec #326): the local static server the docs site's Playwright pass
// reads the site through, as built (ADR 0068; spec #326, ambiguity 5). These
// tests serve a fixture folder, carry no tag and declare no Feature.
import { fileURLToPath } from "node:url";
import { describe, expect, onTestFinished, test } from "vitest";
import { serveStatic, type StaticServer } from "@goodfetch/test-support/serve-static";

const site = fileURLToPath(new URL("fixtures/static-site/", import.meta.url));

/** Serves the fixture site under `/docs/`, stopped when the test finishes. */
async function served(): Promise<StaticServer> {
  const server = await serveStatic(site, "/docs/");
  onTestFinished(() => server.close());
  return server;
}

describe("serveStatic", () => {
  test.for([
    [
      "the home page",
      "",
      "<!doctype html>\n<title>Home</title>\n<p>The fixture site's home page.</p>\n",
    ],
    [
      "a nested page",
      "guide/",
      "<!doctype html>\n<title>Guide</title>\n<p>The fixture site's guide.</p>\n",
    ],
  ])("serves %s of the built folder under the base", async ([, path, page]) => {
    const server = await served();

    const response = await fetch(new URL(path ?? "", server.url));

    expect([response.status, await response.text()]).toEqual([200, page]);
  });

  test("listens on a loopback port the system chose", async () => {
    const server = await served();

    expect(server.url).toMatch(/^http:\/\/127\.0\.0\.1:[1-9][0-9]*\/docs\/$/);
  });

  // `/index.html` is a page of the folder: only the base keeps it out.
  test.for([
    ["the site's root", "/"],
    ["a page of the folder without the base", "/index.html"],
    ["a path beside the base", "/docs-old/index.html"],
  ])("answers 404 for %s", async ([, path]) => {
    const server = await served();

    const response = await fetch(new URL(path ?? "", server.url));

    expect(response.status).toBe(404);
  });

  test("answers 404 for a page the folder lacks under the base", async () => {
    const server = await served();

    const response = await fetch(new URL("missing/", server.url));

    expect(response.status).toBe(404);
  });

  test("gives two servers two ports", async () => {
    const first = await served();
    const second = await served();

    expect(new URL(first.url).port).not.toBe(new URL(second.url).port);
  });

  test("refuses connections once closed", async () => {
    const server = await serveStatic(site, "/docs/");

    await server.close();

    await expect(fetch(server.url)).rejects.toMatchObject({ cause: { code: "ECONNREFUSED" } });
  });
});
