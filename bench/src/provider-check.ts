// Nightly provider shape check (ADRs 0064, 0065 and 0076, ticket #417):
// sends each recorded request live with a Vercel AI Gateway free-tier model
// and compares response shapes (JSON field types, stream event kinds and
// order, error bodies, read headers) and the provider's model list against
// `test-support/fixtures/models.toml`, never content. Writes results under the
// `live` tier and stays disabled until M4 via `#413`'s shared live-gate.
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import {
  INCONCLUSIVE_EXIT_CODE,
  classifyGatewayResponse,
  loadCapsTable,
  runLiveJob,
  type LiveCapsTable,
} from "./live-gate.ts";

/** One recorded request sent to a Model provider. */
export interface RecordedRequest {
  readonly method: string;
  readonly path: string;
  readonly headers: Readonly<Record<string, string>>;
  readonly body?: unknown;
}

/** One recorded response: either a JSON/text body or SSE/JSONL stream chunks. */
export interface RecordedResponse {
  readonly status: number;
  readonly headers: Readonly<Record<string, string>>;
  readonly body?: unknown;
  readonly chunks?: readonly string[] | undefined;
}

/** One recorded Model exchange (matches `goodfetch-test-support`'s `ModelRecording`). */
export interface RecordedExchange {
  readonly request: RecordedRequest;
  readonly response: RecordedResponse;
  readonly synthetic?: boolean | undefined;
}

/** One live response received from the Vercel AI Gateway during the check. */
export interface LiveExchangeResponse {
  readonly status: number;
  readonly headers: Readonly<Record<string, string>>;
  readonly body?: unknown;
  readonly chunks?: readonly string[] | undefined;
  readonly errorCode?: string | undefined;
  readonly retryAfterSeconds?: number | null | undefined;
}

/** Kind of shape difference reported by `provider-check`. */
export type DifferenceKind =
  | "status"
  | "field-type"
  | "stream-event-kind"
  | "stream-event-order"
  | "error-body"
  | "read-header"
  | "missing-model";

/** One structural difference detected against a recording or model manifest. */
export interface ShapeDifference {
  readonly kind: DifferenceKind;
  readonly path: string;
  readonly recorded: string;
  readonly live: string;
}

/** Comparison verdict for a single recorded exchange. */
export interface ExchangeComparison {
  readonly passed: boolean;
  readonly differences: readonly ShapeDifference[];
}

type JsonShape =
  | "null"
  | "boolean"
  | "number"
  | "string"
  | { readonly array: readonly JsonShape[] }
  | { readonly object: Readonly<Record<string, JsonShape>> };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function shapeOf(value: unknown): JsonShape {
  if (value === null || value === undefined) {
    return "null";
  }
  if (typeof value === "boolean") {
    return "boolean";
  }
  if (typeof value === "number") {
    return "number";
  }
  if (typeof value === "string") {
    return "string";
  }
  if (Array.isArray(value)) {
    const unique = new Map<string, JsonShape>();
    for (const item of value) {
      const itemShape = shapeOf(item);
      unique.set(JSON.stringify(itemShape), itemShape);
    }
    const sorted = [...unique.entries()]
      .toSorted(([a], [b]) => a.localeCompare(b))
      .map(([, shape]) => shape);
    return { array: sorted };
  }
  if (isRecord(value)) {
    const object: Record<string, JsonShape> = {};
    for (const key of Object.keys(value).toSorted()) {
      object[key] = shapeOf(value[key]);
    }
    return { object };
  }
  return "string";
}

function isArrayShape(shape: JsonShape): shape is { readonly array: readonly JsonShape[] } {
  return typeof shape === "object" && "array" in shape;
}

function isObjectShape(
  shape: JsonShape,
): shape is { readonly object: Readonly<Record<string, JsonShape>> } {
  return typeof shape === "object" && "object" in shape;
}

function describeShape(shape: JsonShape): string {
  return typeof shape === "string" ? shape : JSON.stringify(shape);
}

function diffShapes(
  path: string,
  recorded: JsonShape,
  live: JsonShape,
  kind: "field-type" | "error-body",
  out: ShapeDifference[],
): void {
  // Matches `proving-ground/tests/it/model_recordings.rs`: null on either side
  // represents an unset optional provider field.
  if (recorded === "null" || live === "null") {
    return;
  }
  if (typeof recorded === "string" && typeof live === "string") {
    if (recorded !== live) {
      out.push({ kind, path, recorded, live });
    }
    return;
  }
  if (isArrayShape(recorded) && isArrayShape(live)) {
    if (recorded.array.length === 0 || live.array.length === 0) {
      return;
    }
    if (recorded.array.length === 1 && live.array.length === 1) {
      const recFirst = recorded.array[0];
      const liveFirst = live.array[0];
      if (recFirst !== undefined && liveFirst !== undefined) {
        diffShapes(`${path}[]`, recFirst, liveFirst, kind, out);
      }
      return;
    }
    const recSet = recorded.array.map(describeShape);
    const liveSet = live.array.map(describeShape);
    if (recSet.join("|") !== liveSet.join("|")) {
      out.push({
        kind,
        path: `${path}[]`,
        recorded: `[${recSet.join(", ")}]`,
        live: `[${liveSet.join(", ")}]`,
      });
    }
    return;
  }
  if (isObjectShape(recorded) && isObjectShape(live)) {
    const keys = new Set([...Object.keys(recorded.object), ...Object.keys(live.object)]);
    for (const key of [...keys].toSorted()) {
      const childPath = path === "$" ? `$.${key}` : `${path}.${key}`;
      const recChild = recorded.object[key];
      const liveChild = live.object[key];
      if (recChild === undefined || liveChild === undefined) {
        out.push({
          kind,
          path: childPath,
          recorded: recChild === undefined ? "<absent>" : describeShape(recChild),
          live: liveChild === undefined ? "<absent>" : describeShape(liveChild),
        });
      } else {
        diffShapes(childPath, recChild, liveChild, kind, out);
      }
    }
    return;
  }
  out.push({
    kind,
    path,
    recorded: describeShape(recorded),
    live: describeShape(live),
  });
}

/** Extracts ordered stream event kinds from SSE / JSONL chunks. */
export function parseStreamEventKinds(chunks: readonly string[]): readonly string[] {
  const joined = chunks.join("");
  const events: string[] = [];
  for (const frame of joined.split(/\r?\n\r?\n/)) {
    const trimmed = frame.trim();
    if (trimmed.length === 0) {
      continue;
    }
    let eventName: string | undefined;
    let dataLine: string | undefined;
    for (const line of trimmed.split(/\r?\n/)) {
      if (line.startsWith("event:")) {
        eventName = line.slice("event:".length).trim();
      } else if (line.startsWith("data:")) {
        dataLine = line.slice("data:".length).trim();
      }
    }
    if (eventName !== undefined && eventName.length > 0) {
      events.push(eventName);
      continue;
    }
    if (dataLine === "[DONE]") {
      events.push("[DONE]");
      continue;
    }
    if (dataLine !== undefined && dataLine.length > 0) {
      try {
        const parsed: unknown = JSON.parse(dataLine);
        if (isRecord(parsed) && typeof parsed["type"] === "string") {
          events.push(parsed["type"]);
        } else {
          events.push("data");
        }
      } catch {
        events.push("data");
      }
    }
  }
  return events;
}

function deduplicateConsecutive(seq: readonly string[]): string[] {
  return seq.filter((kind, index) => index === 0 || seq[index - 1] !== kind);
}

function compareStreamChunks(
  recordedChunks: readonly string[],
  liveChunks: readonly string[],
  out: ShapeDifference[],
): void {
  const recKinds = parseStreamEventKinds(recordedChunks);
  const liveKinds = parseStreamEventKinds(liveChunks);
  const recSet = new Set(recKinds);
  const liveSet = new Set(liveKinds);

  for (const kind of liveSet) {
    if (!recSet.has(kind)) {
      out.push({
        kind: "stream-event-kind",
        path: "stream.events",
        recorded: `[${[...recSet].join(", ")}]`,
        live: `new event kind "${kind}"`,
      });
    }
  }
  for (const kind of recSet) {
    if (!liveSet.has(kind)) {
      out.push({
        kind: "stream-event-kind",
        path: "stream.events",
        recorded: `expected event kind "${kind}"`,
        live: `[${[...liveSet].join(", ")}]`,
      });
    }
  }

  const recOrder = deduplicateConsecutive(recKinds);
  const liveOrder = deduplicateConsecutive(liveKinds);
  if (
    recSet.size === liveSet.size &&
    [...recSet].every((k) => liveSet.has(k)) &&
    recOrder.join(" -> ") !== liveOrder.join(" -> ")
  ) {
    out.push({
      kind: "stream-event-order",
      path: "stream.order",
      recorded: recOrder.join(" -> "),
      live: liveOrder.join(" -> "),
    });
  }
}

function lowercaseHeaders(headers: Readonly<Record<string, string>>): ReadonlyMap<string, string> {
  const map = new Map<string, string>();
  for (const [name, value] of Object.entries(headers)) {
    map.set(name.toLowerCase(), value);
  }
  return map;
}

/** Compares one recorded exchange against a live provider response by shape alone. */
export function compareExchange(
  recorded: RecordedExchange,
  live: LiveExchangeResponse,
): ExchangeComparison {
  const differences: ShapeDifference[] = [];
  const isError = recorded.response.status >= 400 || live.status >= 400;

  if (recorded.response.status !== live.status) {
    differences.push({
      kind: isError ? "error-body" : "status",
      path: "response.status",
      recorded: String(recorded.response.status),
      live: String(live.status),
    });
  }

  const recHeaders = lowercaseHeaders(recorded.response.headers);
  const liveHeaders = lowercaseHeaders(live.headers);
  for (const [name] of recHeaders) {
    if (!liveHeaders.has(name)) {
      differences.push({
        kind: "read-header",
        path: name,
        recorded: "present",
        live: "missing",
      });
    }
  }

  if (recorded.response.chunks !== undefined || live.chunks !== undefined) {
    compareStreamChunks(recorded.response.chunks ?? [], live.chunks ?? [], differences);
  } else {
    diffShapes(
      "$",
      shapeOf(recorded.response.body),
      shapeOf(live.body),
      isError ? "error-body" : "field-type",
      differences,
    );
  }

  return { passed: differences.length === 0, differences };
}

/** Extracts every pinned `model = "..."` reference from `test-support/fixtures/models.toml`. */
export function parseModelsManifest(tomlText: string): readonly string[] {
  const models = new Set<string>();
  const pattern = /\bmodel\s*=\s*"([^"]+)"/g;
  for (const match of tomlText.matchAll(pattern)) {
    const id = match[1];
    if (id !== undefined && id.length > 0) {
      models.add(id);
    }
  }
  return [...models];
}

/** Loads the committed model manifest from `test-support/fixtures/models.toml`. */
export function loadModelsManifest(
  path: string = fileURLToPath(new URL("../../test-support/fixtures/models.toml", import.meta.url)),
): readonly string[] {
  return parseModelsManifest(readFileSync(path, "utf8"));
}

/** Compares every pinned model in the manifest against the provider's listed models. */
export function compareModelManifest(
  manifestModels: readonly string[],
  providerModels: readonly string[],
): readonly ShapeDifference[] {
  const available = new Set(providerModels);
  const missing: ShapeDifference[] = [];
  for (const model of manifestModels) {
    if (!available.has(model)) {
      missing.push({
        kind: "missing-model",
        path: model,
        recorded: "pinned in test-support/fixtures/models.toml",
        live: "absent from provider model list",
      });
    }
  }
  return missing;
}

const SKIPPED_DIRS = new Set([".git", "target", "node_modules", ".venv", "rnd"]);

function parseStringRecord(value: unknown): Readonly<Record<string, string>> {
  if (!isRecord(value)) {
    return {};
  }
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(value)) {
    if (typeof v === "string") {
      out[k] = v;
    }
  }
  return out;
}

function parseRecordedExchange(raw: unknown): RecordedExchange | null {
  if (!isRecord(raw) || !isRecord(raw["request"]) || !isRecord(raw["response"])) {
    return null;
  }
  const req = raw["request"];
  const resp = raw["response"];
  if (
    typeof req["method"] !== "string" ||
    typeof req["path"] !== "string" ||
    typeof resp["status"] !== "number"
  ) {
    return null;
  }
  const chunksRaw = resp["chunks"];
  const chunks =
    Array.isArray(chunksRaw) && chunksRaw.every((c) => typeof c === "string")
      ? chunksRaw
      : undefined;
  return {
    request: {
      method: req["method"],
      path: req["path"],
      headers: parseStringRecord(req["headers"]),
      body: req["body"],
    },
    response: {
      status: resp["status"],
      headers: parseStringRecord(resp["headers"]),
      body: resp["body"],
      chunks,
    },
    synthetic: raw["synthetic"] === true,
  };
}

/** Discovers every committed Model recording (`recordings/<file>.json`) under `repoRoot`. */
export function discoverRecordings(
  repoRoot: string = fileURLToPath(new URL("../..", import.meta.url)),
): readonly NamedRecording[] {
  const found: NamedRecording[] = [];
  function walk(dir: string, insideRecordings: boolean): void {
    for (const entry of readdirSync(dir, { withFileTypes: true }).toSorted((a, b) =>
      a.name.localeCompare(b.name),
    )) {
      if (entry.isDirectory()) {
        if (!SKIPPED_DIRS.has(entry.name)) {
          walk(join(dir, entry.name), insideRecordings || entry.name === "recordings");
        }
      } else if (insideRecordings && entry.name.toLowerCase().endsWith(".json")) {
        const fullPath = join(dir, entry.name);
        const rel = relative(repoRoot, fullPath).replaceAll("\\", "/");
        const parsed: unknown = JSON.parse(readFileSync(fullPath, "utf8"));
        const rawList =
          isRecord(parsed) && Array.isArray(parsed["exchanges"])
            ? parsed["exchanges"]
            : Array.isArray(parsed)
              ? parsed
              : [parsed];
        for (const [index, item] of rawList.entries()) {
          const exchange = parseRecordedExchange(item);
          if (exchange !== null) {
            found.push({ name: `${rel}#${index + 1}`, exchange });
          }
        }
      }
    }
  }
  walk(repoRoot, false);
  return found;
}

/** Builds the live HTTP transport against Vercel AI Gateway for `provider-check`. */
export function createGatewayTransport(
  gatewayBaseUrl: string,
  apiKey: string,
): {
  readonly sendExchange: (
    exchange: RecordedExchange,
    model: string,
  ) => Promise<LiveExchangeResponse>;
  readonly listModels: () => Promise<readonly string[]>;
} {
  const base = gatewayBaseUrl.replace(/\/+$/, "");
  return {
    sendExchange: async (exchange, model) => {
      const url = `${base}${exchange.request.path}`;
      const headers: Record<string, string> = {
        ...exchange.request.headers,
        authorization: `Bearer ${apiKey}`,
      };
      const rawBody = isRecord(exchange.request.body)
        ? { ...exchange.request.body, model }
        : exchange.request.body;
      const response = await fetch(url, {
        method: exchange.request.method,
        headers,
        ...(rawBody !== undefined ? { body: JSON.stringify(rawBody) } : {}),
      });
      const respHeaders: Record<string, string> = {};
      response.headers.forEach((value, key) => {
        respHeaders[key.toLowerCase()] = value;
      });
      const retryHeader = respHeaders["retry-after"];
      const retryAfterSeconds =
        retryHeader !== undefined && Number.isFinite(Number(retryHeader))
          ? Number(retryHeader)
          : null;
      const contentType = respHeaders["content-type"] ?? "";
      const text = await response.text();
      if (contentType.includes("text/event-stream")) {
        return {
          status: response.status,
          headers: respHeaders,
          chunks: [text],
          retryAfterSeconds,
        };
      }
      let body: unknown = text;
      let errorCode: string | undefined;
      try {
        body = JSON.parse(text);
        if (
          isRecord(body) &&
          isRecord(body["error"]) &&
          typeof body["error"]["code"] === "string"
        ) {
          errorCode = body["error"]["code"];
        }
      } catch {
        // Non-JSON response preserved as text.
      }
      return {
        status: response.status,
        headers: respHeaders,
        body,
        errorCode,
        retryAfterSeconds,
      };
    },
    listModels: async () => {
      const response = await fetch(`${base}/v1/models`, {
        headers: { authorization: `Bearer ${apiKey}` },
      });
      if (!response.ok) {
        return [];
      }
      const parsed: unknown = await response.json();
      if (!isRecord(parsed) || !Array.isArray(parsed["data"])) {
        return [];
      }
      const ids: string[] = [];
      for (const item of parsed["data"]) {
        if (isRecord(item) && typeof item["id"] === "string") {
          ids.push(item["id"]);
        }
      }
      return ids;
    },
  };
}

function escapeXml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

/** Renders a JUnit XML document for `provider-check` under the `live` tier. */
export function renderProviderCheckJunit(input: {
  readonly tier: "live";
  readonly status: "passed" | "failed" | "inconclusive" | "disabled";
  readonly differences: readonly ShapeDifference[];
  readonly note?: string | undefined;
}): string {
  const failures = input.status === "failed" ? 1 : 0;
  const skipped = input.status === "inconclusive" ? 1 : 0;
  const lines = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    `<testsuite name="${input.tier}" tests="1" failures="${failures}" skipped="${skipped}">`,
    '  <testcase classname="live" name="provider-check">',
  ];
  if (input.status === "failed") {
    const detail = input.differences
      .map((d) => `[${d.kind}] ${d.path}: recorded=${d.recorded} live=${d.live}`)
      .join("\n");
    lines.push(
      `    <failure message="${escapeXml(`${input.differences.length} provider shape difference(s)`)}">${escapeXml(detail)}</failure>`,
    );
  } else if (input.status === "inconclusive") {
    lines.push(
      `    <skipped message="${escapeXml(input.note ?? "provider check inconclusive")}"/>`,
    );
  }
  lines.push("  </testcase>", "</testsuite>", "");
  return lines.join("\n");
}

/** Named recording input passed to `runProviderCheck`. */
export interface NamedRecording {
  readonly name: string;
  readonly exchange: RecordedExchange;
}

/** Options for `runProviderCheck` (Seam D accepts scripted `sendExchange` / `listModels`). */
export interface ProviderCheckOptions {
  readonly env?: Readonly<Record<string, string | undefined>> | undefined;
  readonly caps?: LiveCapsTable | undefined;
  readonly model?: string | undefined;
  readonly recordings: readonly NamedRecording[];
  readonly manifestModels: readonly string[];
  readonly junitPath?: string | undefined;
  readonly writeLog?: ((line: string) => void) | undefined;
  readonly sendExchange: (
    exchange: RecordedExchange,
    model: string,
  ) => Promise<LiveExchangeResponse>;
  readonly listModels: () => Promise<readonly string[]>;
}

/** Structured verdict of `runProviderCheck`. */
export interface ProviderCheckOutcome {
  readonly tier: "live";
  readonly status: "disabled" | "passed" | "failed" | "inconclusive" | "refused_over_cap";
  readonly exitCode: number;
  readonly differences: readonly ShapeDifference[];
  readonly note?: string | undefined;
}

const CHECK_INPUT_TOKENS = 2_000;
const CHECK_OUTPUT_TOKENS = 500;

/** Runs ADR 0065's `provider-check` campaign under `#413`'s shared live-gate. */
export async function runProviderCheck(
  options: ProviderCheckOptions,
): Promise<ProviderCheckOutcome> {
  const model = options.model ?? "openai/gpt-5.4-nano";
  const caps = options.caps ?? loadCapsTable();
  const nonSynthetic = options.recordings.filter((item) => item.exchange.synthetic !== true);
  const price = caps.models[model];
  const perCallCostUsd =
    price !== undefined
      ? (CHECK_INPUT_TOKENS * price.inputPerMillionUsd +
          CHECK_OUTPUT_TOKENS * price.outputPerMillionUsd) /
        1_000_000
      : 0.01;

  const jobOutcome = await runLiveJob<{
    readonly differences: readonly ShapeDifference[];
  }>({
    job: "provider-check",
    env: options.env,
    caps,
    writeLog: options.writeLog,
    trials: [
      {
        model,
        count: Math.max(1, nonSynthetic.length),
        maxInputTokens: CHECK_INPUT_TOKENS,
        maxOutputTokens: CHECK_OUTPUT_TOKENS,
      },
    ],
    execute: async (ctx) => {
      const differences: ShapeDifference[] = [];
      for (const item of nonSynthetic) {
        const live = await options.sendExchange(item.exchange, model);
        const signal = {
          status: live.status,
          errorCode: live.errorCode,
          body: typeof live.body === "string" ? live.body : JSON.stringify(live.body ?? ""),
          retryAfterSeconds: live.retryAfterSeconds,
        };
        const classified = classifyGatewayResponse(signal);
        if (classified.kind === "inconclusive") {
          return ctx.inconclusiveFromGateway(signal);
        }
        ctx.recordCall(perCallCostUsd);
        const compared = compareExchange(item.exchange, live);
        for (const diff of compared.differences) {
          differences.push({ ...diff, path: `${item.name}:${diff.path}` });
        }
      }
      const listed = await options.listModels();
      differences.push(...compareModelManifest(options.manifestModels, listed));
      return { differences };
    },
  });

  let outcome: ProviderCheckOutcome;
  if (jobOutcome.status === "disabled") {
    outcome = { tier: "live", status: "disabled", exitCode: 0, differences: [] };
  } else if (jobOutcome.status === "inconclusive") {
    outcome = {
      tier: "live",
      status: "inconclusive",
      exitCode: INCONCLUSIVE_EXIT_CODE,
      differences: [],
      note: jobOutcome.reason,
    };
  } else if (jobOutcome.status === "refused_over_cap" || jobOutcome.status === "stopped_at_limit") {
    outcome = {
      tier: "live",
      status: "refused_over_cap",
      exitCode: 1,
      differences: [],
      note: "provider-check cap exceeded",
    };
  } else if (jobOutcome.value.differences.length > 0) {
    outcome = {
      tier: "live",
      status: "failed",
      exitCode: 1,
      differences: jobOutcome.value.differences,
    };
  } else {
    outcome = { tier: "live", status: "passed", exitCode: 0, differences: [] };
  }

  if (options.junitPath !== undefined) {
    mkdirSync(dirname(options.junitPath), { recursive: true });
    writeFileSync(
      options.junitPath,
      renderProviderCheckJunit({
        tier: "live",
        status: outcome.status === "refused_over_cap" ? "failed" : outcome.status,
        differences: outcome.differences,
        note: outcome.note,
      }),
      "utf8",
    );
  }
  return outcome;
}
