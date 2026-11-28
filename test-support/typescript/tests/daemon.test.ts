// Seam B (spec #325): the `daemon` fixture, used exactly as a product test
// uses it, against the real `goodfetch-test-env --daemon` and the commit's
// Collie or Foxhound executable, which the tool manifest names. A Client
// reaches the Daemon of the test's own home through the described socket
// with the described token.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect } from "vitest";
import { test, startTestEnv } from "@goodfetch/test-support";
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

const DAEMON_STATUS_BODY = JSON.stringify({
  jsonrpc: "2.0",
  id: 1,
  method: "daemon.status",
  params: {},
});

const HTTP_RPC_BASE_HEADERS = {
  "Content-Type": "application/json",
  "Goodfetch-Protocol": "0.1",
  "Goodfetch-Product": "collie",
  "Goodfetch-Form": "catalog",
};

describe("daemon with HTTP listener", () => {
  test.override({ withDaemon: true, withHttpListener: true });

  test("answers daemon.status over HTTP with its Bearer token", async ({ daemon }) => {
    expect(daemon.http).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
    expect(daemon.http_token).toBeTruthy();

    const response = await fetch(`${daemon.http}/v1/rpc`, {
      method: "POST",
      headers: { ...HTTP_RPC_BASE_HEADERS, Authorization: `Bearer ${daemon.http_token}` },
      body: DAEMON_STATUS_BODY,
    });

    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({ result: { product: "collie" } });
  });

  test("refuses HTTP requests without an Authorization header", async ({ daemon }) => {
    const response = await fetch(`${daemon.http}/v1/rpc`, {
      method: "POST",
      headers: HTTP_RPC_BASE_HEADERS,
      body: DAEMON_STATUS_BODY,
    });

    expect(response.status).toBe(401);
  });

  test("refuses HTTP requests carrying the socket token instead of http_token", async ({
    daemon,
  }) => {
    const response = await fetch(`${daemon.http}/v1/rpc`, {
      method: "POST",
      headers: { ...HTTP_RPC_BASE_HEADERS, Authorization: `Bearer ${daemon.token}` },
      body: DAEMON_STATUS_BODY,
    });

    expect(response.status).toBe(401);
  });
});

describe("httpListener without daemon", () => {
  test("fails goodfetch-test-env start with start_failed", async ({ processes, toolManifest }) => {
    await expect(
      startTestEnv({
        processes,
        testName: "httpListener without daemon",
        toolManifest,
        httpListener: true,
      }),
    ).rejects.toMatchObject({ code: "start_failed" });
  });
});
