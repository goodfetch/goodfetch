// Seam B (spec #326 / #426; ADR 0070): pinned n8n and Activepieces images
// in `ci/integrations/images.toml`, each running under `--network=none` and
// reaching `goodfetch-test-env --daemon --http-listener` with its Bearer token,
// and failing when the token is absent/wrong or an image is unpulled.
import { describe, expect } from "vitest";
import { runN8nWorkflow, startActivepieces, test } from "@goodfetch/test-support";

function n8nDaemonStatusWorkflow(id: string, daemonHttp: string, bearerToken: string) {
  return {
    id,
    name: "Goodfetch Daemon HTTP Listener Probe",
    nodes: [
      {
        id: "1",
        name: "Start",
        type: "n8n-nodes-base.manualTrigger",
        typeVersion: 1,
        position: [0, 0],
        parameters: {},
      },
      {
        id: "2",
        name: "Daemon Status",
        type: "n8n-nodes-base.httpRequest",
        typeVersion: 4.2,
        position: [220, 0],
        parameters: {
          method: "POST",
          url: `${daemonHttp}/v1/rpc`,
          sendHeaders: true,
          headerParameters: {
            parameters: [
              { name: "Authorization", value: `Bearer ${bearerToken}` },
              { name: "Goodfetch-Protocol", value: "0.1" },
              { name: "Goodfetch-Product", value: "collie" },
              { name: "Goodfetch-Form", value: "catalog" },
            ],
          },
          sendBody: true,
          specifyBody: "json",
          jsonBody: JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            method: "daemon.status",
            params: {},
          }),
          options: {},
        },
      },
    ],
    connections: {
      Start: { main: [[{ node: "Daemon Status", type: "main", index: 0 }]] },
    },
    active: false,
    settings: {},
  };
}

const ACTIVEPIECES_RPC_CODE = [
  "export const code = async (inputs) => {",
  "  const headers = {",
  '    "Content-Type": "application/json",',
  '    "Goodfetch-Protocol": "0.1",',
  '    "Goodfetch-Product": "collie",',
  '    "Goodfetch-Form": "catalog",',
  "  };",
  "  if (inputs.token) {",
  '    headers["Authorization"] = `Bearer ${inputs.token}`;',
  "  }",
  "  const res = await fetch(`${inputs.url}/v1/rpc`, {",
  '    method: "POST",',
  "    headers,",
  '    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "daemon.status", params: {} }),',
  "  });",
  "  let product = null;",
  "  if (res.ok) {",
  "    const data = await res.json();",
  "    product = data.result?.product ?? null;",
  "  }",
  "  return { status: res.status, product };",
  "};",
].join("\n");

const UNPULLED_IMAGE = `docker.io/n8nio/n8n@sha256:${"0".repeat(64)}`;

describe.skipIf(process.platform !== "linux")("n8n and Activepieces integration harnesses", () => {
  test.override({ withDaemon: true, withHttpListener: true });

  test("runN8nWorkflow refuses an unpulled image with container_image_missing", async () => {
    await expect(
      runN8nWorkflow({
        workflowId: "goodfetch-unpulled",
        workflow: n8nDaemonStatusWorkflow("goodfetch-unpulled", "http://127.0.0.1:1", "t"),
        image: UNPULLED_IMAGE,
      }),
    ).rejects.toMatchObject({ code: "container_image_missing" });
  });

  test("startActivepieces refuses an unpulled image with container_image_missing", async () => {
    await expect(startActivepieces({ image: UNPULLED_IMAGE })).rejects.toMatchObject({
      code: "container_image_missing",
    });
  });

  test("fixture n8n workflow reaches the Daemon HTTP listener with its Bearer token", async ({
    daemon,
  }) => {
    const daemonHttp = daemon.http;
    const httpToken = daemon.http_token;
    expect(daemonHttp).toBeDefined();
    expect(httpToken).toBeDefined();
    if (daemonHttp === undefined || httpToken === undefined) {
      return;
    }

    const result = await runN8nWorkflow({
      workflowId: "goodfetch-n8n-ok",
      daemonHttp,
      workflow: n8nDaemonStatusWorkflow("goodfetch-n8n-ok", daemonHttp, httpToken),
    });

    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain('"product": "collie"');
  }, 180_000);

  test("fixture n8n workflow fails with 401 when given a wrong Bearer token", async ({
    daemon,
  }) => {
    const daemonHttp = daemon.http;
    expect(daemonHttp).toBeDefined();
    if (daemonHttp === undefined) {
      return;
    }

    const result = await runN8nWorkflow({
      workflowId: "goodfetch-n8n-bad",
      daemonHttp,
      workflow: n8nDaemonStatusWorkflow("goodfetch-n8n-bad", daemonHttp, "wrong-token"),
    });

    expect(result.exitCode).not.toBe(0);
    expect(`${result.stdout}\n${result.stderr}`).toContain("401");
  }, 180_000);

  test("fixture Activepieces workflow reaches the Daemon HTTP listener with its Bearer token", async ({
    daemon,
  }) => {
    const daemonHttp = daemon.http;
    const httpToken = daemon.http_token;
    expect(daemonHttp).toBeDefined();
    expect(httpToken).toBeDefined();
    if (daemonHttp === undefined || httpToken === undefined) {
      return;
    }

    const ap = await startActivepieces({ daemonHttp });
    try {
      const result = await ap.runCodeStep({
        code: ACTIVEPIECES_RPC_CODE,
        input: { url: daemonHttp, token: httpToken },
      });

      expect(result.status).toBe("SUCCEEDED");
      expect(result.output).toMatchObject({ status: 200, product: "collie" });
    } finally {
      await ap.stop();
    }
  }, 180_000);

  test("fixture Activepieces workflow receives 401 when the Bearer token is missing", async ({
    daemon,
  }) => {
    const daemonHttp = daemon.http;
    expect(daemonHttp).toBeDefined();
    if (daemonHttp === undefined) {
      return;
    }

    const ap = await startActivepieces({ daemonHttp });
    try {
      const result = await ap.runCodeStep({
        code: ACTIVEPIECES_RPC_CODE,
        input: { url: daemonHttp, token: "" },
      });

      expect(result.status).toBe("SUCCEEDED");
      expect(result.output).toMatchObject({ status: 401, product: null });
    } finally {
      await ap.stop();
    }
  }, 180_000);
});
