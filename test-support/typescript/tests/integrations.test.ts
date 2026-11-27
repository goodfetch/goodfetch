// Seam B (spec #326 / #426; ADR 0070): pinned n8n and Activepieces images
// in `ci/integrations/images.toml`, each reaching `goodfetch-test-env --daemon
// --http-listener` with its Bearer token and failing when the token is absent
// or wrong.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect } from "vitest";
import {
  TestSupportError,
  detectContainerEngine,
  readIntegrationImages,
  runN8nWorkflow,
  startActivepieces,
  test,
} from "@goodfetch/test-support";

describe("integration image pins", () => {
  test("reads pinned n8n and Activepieces digests from ci/integrations/images.toml", () => {
    const images = readIntegrationImages();

    expect(images.n8n.reference).toMatch(/^docker\.io\/n8nio\/n8n@sha256:[0-9a-f]{64}$/);
    expect(images.activepieces.reference).toMatch(
      /^docker\.io\/activepieces\/activepieces@sha256:[0-9a-f]{64}$/,
    );
  });

  test("refuses an images.toml whose entry lacks a sha256 digest", () => {
    const folder = mkdtempSync(join(tmpdir(), "goodfetch-pins-"));
    try {
      const badFile = join(folder, "images.toml");
      writeFileSync(
        badFile,
        '[n8n]\nrepository = "docker.io/n8nio/n8n"\ntag = "2.50.3"\n\n[activepieces]\nrepository = "docker.io/activepieces/activepieces"\ntag = "0.94.0"\ndigest = "sha256:bad"\n',
        "utf8",
      );
      expect(() => readIntegrationImages(badFile)).toThrow(TestSupportError);
      let caught: unknown;
      try {
        readIntegrationImages(badFile);
      } catch (error) {
        caught = error;
      }
      expect(caught).toBeInstanceOf(TestSupportError);
      expect(caught).toMatchObject({ code: "integration_images_invalid" });
    } finally {
      rmSync(folder, { recursive: true, force: true });
    }
  });
});

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

const canRunLinuxContainers = process.platform === "linux" && detectContainerEngine() !== undefined;

describe.skipIf(!canRunLinuxContainers)("integration harness Seam B", () => {
  test.override({ withDaemon: true, withHttpListener: true });

  test("fixture n8n workflow reaches the Daemon HTTP listener and fails on a wrong token", async ({
    daemon,
  }) => {
    const daemonHttp = daemon.http;
    const httpToken = daemon.http_token;
    expect(daemonHttp).toBeDefined();
    expect(httpToken).toBeDefined();
    if (daemonHttp === undefined || httpToken === undefined) {
      return;
    }

    const good = await runN8nWorkflow({
      workflowId: "goodfetch-n8n-ok",
      workflow: n8nDaemonStatusWorkflow("goodfetch-n8n-ok", daemonHttp, httpToken),
    });
    expect(good.exitCode).toBe(0);
    expect(good.stdout).toContain('"product": "collie"');

    const bad = await runN8nWorkflow({
      workflowId: "goodfetch-n8n-bad",
      workflow: n8nDaemonStatusWorkflow("goodfetch-n8n-bad", daemonHttp, "wrong-token"),
    });
    expect(bad.exitCode).not.toBe(0);
  }, 180_000);

  test("fixture Activepieces single-container workflow reaches the Daemon HTTP listener and observes 401 without token", async ({
    daemon,
  }) => {
    const daemonHttp = daemon.http;
    const httpToken = daemon.http_token;
    expect(daemonHttp).toBeDefined();
    expect(httpToken).toBeDefined();
    if (daemonHttp === undefined || httpToken === undefined) {
      return;
    }

    const ap = await startActivepieces();
    try {
      const code = [
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

      const good = await ap.runCodeStep({
        code,
        input: { url: daemonHttp, token: httpToken },
      });
      expect(good.status).toBe("SUCCEEDED");
      expect(good.output).toMatchObject({ status: 200, product: "collie" });

      const unauthorized = await ap.runCodeStep({
        code,
        input: { url: daemonHttp, token: "" },
      });
      expect(unauthorized.status).toBe("SUCCEEDED");
      expect(unauthorized.output).toMatchObject({ status: 401, product: null });
    } finally {
      await ap.stop();
    }
  }, 180_000);
});
