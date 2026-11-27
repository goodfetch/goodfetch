import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "vitest";
import {
  compareExchange,
  compareModelManifest,
  parseModelsManifest,
  renderProviderCheckJunit,
  runProviderCheck,
  type LiveExchangeResponse,
  type RecordedExchange,
} from "./provider-check.ts";

const FAKE_GATEWAY_KEY = ["vck", "test", "00000000000000000000"].join("_");

const RECORDED_TEXT: RecordedExchange = {
  request: {
    method: "POST",
    path: "/v1/chat/completions",
    headers: { "content-type": "application/json" },
    body: { model: "openai/gpt-5.4-nano", messages: [{ role: "user", content: "Say hello." }] },
  },
  response: {
    status: 200,
    headers: {
      "content-type": "application/json",
      "x-ratelimit-remaining-requests": "99",
    },
    body: {
      id: "chatcmpl-1",
      choices: [{ index: 0, message: { role: "assistant", content: "Hello!" } }],
      usage: { prompt_tokens: 10, completion_tokens: 5 },
    },
  },
};

const RECORDED_STREAM: RecordedExchange = {
  request: {
    method: "POST",
    path: "/v1/messages",
    headers: { "content-type": "application/json", "anthropic-version": "2023-06-01" },
    body: { model: "openai/gpt-5.4-nano", stream: true },
  },
  response: {
    status: 200,
    headers: { "content-type": "text/event-stream" },
    chunks: [
      'event: message_start\ndata: {"type":"message_start"}\n\n',
      'event: content_block_delta\ndata: {"type":"content_block_delta","delta":{"text":"Hi"}}\n\n',
      'event: message_stop\ndata: {"type":"message_stop"}\n\n',
    ],
  },
};

const RECORDED_ERROR: RecordedExchange = {
  request: {
    method: "POST",
    path: "/v1/chat/completions",
    headers: { "content-type": "application/json" },
    body: { model: "openai/gpt-5.4-nano" },
  },
  response: {
    status: 400,
    headers: { "content-type": "application/json" },
    body: {
      error: {
        message: "invalid parameter",
        type: "invalid_request_error",
        code: "invalid_value",
      },
    },
  },
};

test("identical shapes with completely different content pass", () => {
  const live: LiveExchangeResponse = {
    status: 200,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "x-ratelimit-remaining-requests": "42",
      "x-unrelated-extra": "ignored",
    },
    body: {
      id: "chatcmpl-99999",
      choices: [
        {
          index: 0,
          message: { role: "assistant", content: "Completely different prose response!" },
        },
      ],
      usage: { prompt_tokens: 123, completion_tokens: 456 },
    },
  };
  const result = compareExchange(RECORDED_TEXT, live);
  expect(result.passed).toBe(true);
  expect(result.differences).toEqual([]);
});

test("reports a changed field type in JSON body", () => {
  const live: LiveExchangeResponse = {
    status: 200,
    headers: {
      "content-type": "application/json",
      "x-ratelimit-remaining-requests": "99",
    },
    body: {
      id: "chatcmpl-2",
      choices: [{ index: "zero", message: { role: "assistant", content: "Hello" } }],
      usage: { prompt_tokens: 10, completion_tokens: 5 },
    },
  };
  const result = compareExchange(RECORDED_TEXT, live);
  expect(result.passed).toBe(false);
  expect(result.differences.some((d) => d.kind === "field-type")).toBe(true);
});

test("reports a missing read header", () => {
  const live: LiveExchangeResponse = {
    status: 200,
    headers: { "content-type": "application/json" },
    body: RECORDED_TEXT.response.body,
  };
  const result = compareExchange(RECORDED_TEXT, live);
  expect(result.passed).toBe(false);
  expect(
    result.differences.some(
      (d) => d.kind === "read-header" && d.path === "x-ratelimit-remaining-requests",
    ),
  ).toBe(true);
});

test("reports a new stream event kind and a reordered stream event", () => {
  const newEventLive: LiveExchangeResponse = {
    status: 200,
    headers: { "content-type": "text/event-stream" },
    chunks: [
      "event: message_start\ndata: {}\n\n",
      "event: thinking_delta\ndata: {}\n\n",
      "event: content_block_delta\ndata: {}\n\n",
      "event: message_stop\ndata: {}\n\n",
    ],
  };
  const newEventResult = compareExchange(RECORDED_STREAM, newEventLive);
  expect(newEventResult.passed).toBe(false);
  expect(newEventResult.differences.some((d) => d.kind === "stream-event-kind")).toBe(true);

  const reorderedLive: LiveExchangeResponse = {
    status: 200,
    headers: { "content-type": "text/event-stream" },
    chunks: [
      "event: content_block_delta\ndata: {}\n\n",
      "event: message_start\ndata: {}\n\n",
      "event: message_stop\ndata: {}\n\n",
    ],
  };
  const reorderedResult = compareExchange(RECORDED_STREAM, reorderedLive);
  expect(reorderedResult.passed).toBe(false);
  expect(reorderedResult.differences.some((d) => d.kind === "stream-event-order")).toBe(true);
});

test("reports a changed error body shape or status", () => {
  const changedErrorLive: LiveExchangeResponse = {
    status: 400,
    headers: { "content-type": "application/json" },
    body: {
      detail: "flat error string instead of error object",
    },
  };
  const result = compareExchange(RECORDED_ERROR, changedErrorLive);
  expect(result.passed).toBe(false);
  expect(result.differences.some((d) => d.kind === "error-body")).toBe(true);
});

test("reports a model from models.toml missing from provider model list", () => {
  const manifestModels = parseModelsManifest(`
[openai-compatible]
small = { model = "openai/gpt-5.4-nano", retires = 2027-04-01, source = "https://example.test" }
larger = { model = "openai/gpt-5.2", source = "https://example.test" }
`);
  expect(manifestModels).toEqual(["openai/gpt-5.4-nano", "openai/gpt-5.2"]);

  const ok = compareModelManifest(manifestModels, [
    "openai/gpt-5.4-nano",
    "openai/gpt-5.2",
    "other",
  ]);
  expect(ok).toEqual([]);

  const missing = compareModelManifest(manifestModels, ["openai/gpt-5.4-nano"]);
  expect(missing).toHaveLength(1);
  expect(missing[0]?.kind).toBe("missing-model");
  expect(missing[0]?.path).toBe("openai/gpt-5.2");
});

test("runProviderCheck stays disabled until #425 and writes JUnit under the live tier when enabled", async () => {
  let fetched = 0;
  const disabled = await runProviderCheck({
    env: {},
    recordings: [{ name: "text.json", exchange: RECORDED_TEXT }],
    manifestModels: ["openai/gpt-5.4-nano"],
    sendExchange: async () => {
      fetched += 1;
      return { status: 200, headers: {}, body: {} };
    },
    listModels: async () => ["openai/gpt-5.4-nano"],
  });
  expect(disabled.status).toBe("disabled");
  expect(disabled.exitCode).toBe(0);
  expect(fetched).toBe(0);

  const dir = mkdtempSync(join(tmpdir(), "gf-provider-check-"));
  try {
    const junitPath = join(dir, "live.xml");
    const failed = await runProviderCheck({
      env: { GOODFETCH_LIVE_PROVIDER_CHECK: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
      recordings: [{ name: "text.json", exchange: RECORDED_TEXT }],
      manifestModels: ["openai/gpt-5.4-nano", "openai/gpt-5.2"],
      junitPath,
      writeLog: () => {},
      sendExchange: async () => ({
        status: 200,
        headers: {
          "content-type": "application/json",
          "x-ratelimit-remaining-requests": "50",
        },
        body: {
          id: "chatcmpl-live",
          choices: [{ index: "wrong-type", message: { role: "assistant", content: "Hi" } }],
          usage: { prompt_tokens: 1, completion_tokens: 1 },
        },
      }),
      listModels: async () => ["openai/gpt-5.4-nano"],
    });
    expect(failed.status).toBe("failed");
    expect(failed.exitCode).toBe(1);
    expect(failed.tier).toBe("live");
    expect(failed.differences.length).toBeGreaterThanOrEqual(2);
    const xml = readFileSync(junitPath, "utf8");
    expect(xml).toContain('testsuite name="live"');
    expect(xml).toContain('<testcase classname="live" name="provider-check">');
    expect(xml).toContain("<failure");
    expect(xml).toContain("openai/gpt-5.2");

    const inconclusiveXml = renderProviderCheckJunit({
      tier: "live",
      status: "inconclusive",
      differences: [],
      note: "rate_limit (HTTP 429)",
    });
    expect(inconclusiveXml).toContain("<skipped");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
