// Seam B (spec #326): test support's Playwright `daemon` fixture, used as a
// served Console test uses it, against the real `goodfetch-test-env
// --daemon` and the commit's Collie or Foxhound executable. Until area 12's
// Console is built, the Console it serves is the stand-in of
// `console-stand-in.ts`, whose home page names the Product that answered a
// `hello` with the Daemon's token.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "@goodfetch/test-support/playwright";
import { standInConsole } from "./console-stand-in.ts";

test.use({ consoleServer: standInConsole() });

test("serves the test's Console beside Collie's Daemon by default", async ({ daemon, page }) => {
  await page.goto(daemon.url);

  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Collie's Daemon");
});

test("gives the Console's own port, on the loopback address", ({ daemon }) => {
  expect(new URL(daemon.url).host).toBe(`127.0.0.1:${daemon.port}`);
});

test("starts the Daemon in the test's own home", ({ daemon }) => {
  const runtimeFile = JSON.parse(
    readFileSync(join(daemon.home, "0.0", "daemon.json"), "utf8"),
  ) as unknown;

  expect(runtimeFile).toMatchObject({ token: daemon.token });
});

test.describe("for a screen only Foxhound carries", () => {
  test.use({ product: "foxhound" });

  test("serves the Console beside Foxhound's Daemon", async ({ daemon, page }) => {
    await page.goto(daemon.url);

    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Foxhound's Daemon");
  });
});

test.describe("with a Proving ground", () => {
  test.use({ provingGroundSeed: 7 });

  test("gives the test the Proving ground of its seed", ({ daemon }) => {
    expect(daemon.provingGround?.seed).toBe(7);
  });
});
