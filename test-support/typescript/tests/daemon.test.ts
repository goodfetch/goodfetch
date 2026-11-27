// Seam B (spec #325): the `daemon` fixture, used exactly as a product test
// uses it, against the real `goodfetch-test-env --daemon` and the commit's
// Collie or Foxhound executable, which the tool manifest names. A Client
// reaches the Daemon of the test's own home through the described socket
// with the described token.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";
import { hello } from "./daemon-hello.ts";

describe("daemon", () => {
  test.override("withDaemon", true);

  test("is Collie's Daemon, reached through its socket with its token", async ({ daemon }) => {
    const answer = await hello(daemon, daemon.token, "collie");

    expect(answer).toMatchObject({ result: { product: "collie" } });
  });

  test("refuses a hello without its token", async ({ daemon }) => {
    const answer = await hello(daemon, `${daemon.token}0`, "collie");

    expect(answer).toBeUndefined();
  });

  test("is the Daemon whose runtime file is in the test's own home", ({ daemon, testEnv }) => {
    const runtimeFile = JSON.parse(
      readFileSync(join(testEnv.home, "0.0", "daemon.json"), "utf8"),
    ) as unknown;

    expect(runtimeFile).toMatchObject({ token: daemon.token });
  });
});

describe("daemon of a test that needs Foxhound", () => {
  test.override({ withDaemon: true, product: "foxhound" });

  test("is run by Foxhound's executable", async ({ daemon }) => {
    const answer = await hello(daemon, daemon.token, "foxhound");

    expect(answer).toMatchObject({ result: { product: "foxhound" } });
  });
});

describe("daemon with HTTP listener", () => {
  test.override({ withDaemon: true, withHttpListener: true });

  test("reaches the HTTP listener with its Bearer token and refuses requests without it", async ({
    daemon,
  }) => {
    expect(daemon.http).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
    expect(daemon.http_token).toBeTruthy();

    const body = JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "daemon.status",
      params: {},
    });
    const baseHeaders = {
      "Content-Type": "application/json",
      "Goodfetch-Protocol": "0.1",
      "Goodfetch-Product": "collie",
      "Goodfetch-Form": "catalog",
    };

    const ok = await fetch(`${daemon.http}/v1/rpc`, {
      method: "POST",
      headers: { ...baseHeaders, Authorization: `Bearer ${daemon.http_token}` },
      body,
    });
    expect(ok.status).toBe(200);
    expect(await ok.json()).toMatchObject({ result: { product: "collie" } });

    const missing = await fetch(`${daemon.http}/v1/rpc`, {
      method: "POST",
      headers: baseHeaders,
      body,
    });
    expect(missing.status).toBe(401);

    const wrongToken = await fetch(`${daemon.http}/v1/rpc`, {
      method: "POST",
      headers: { ...baseHeaders, Authorization: `Bearer ${daemon.token}` },
      body,
    });
    expect(wrongToken.status).toBe(401);
  });
});
