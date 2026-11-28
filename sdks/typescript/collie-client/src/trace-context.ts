// The Trace context each request carries (ADR 0132): the host program's
// active OpenTelemetry context, read through `@opentelemetry/api` when that
// optional peer dependency is installed, so that the program's trace
// continues in the engine; and the reverse for incoming requests from the
// engine to an SDK handler, making the engine's Trace context active around
// the handler so its spans nest under the engine's. Without the package, no
// request carries one and nothing fails. The package is loaded once, before
// a Connection's `hello`, and read as each call is made, in the caller's
// context. Only `traceparent` and `tracestate` are sent or read, never
// Baggage, in the request's `_meta`, which no public type names. The SDK
// opens no span of its own. The Connection's own Subscription, shared by
// every stream of the events it publishes, belongs to no one caller, so its
// request carries none.
import { isRecord, META } from "./framing.ts";

/**
 * The OpenTelemetry API's package, named in a value so that neither the
 * type checker nor a bundler resolves it: a program that does not install
 * it builds and runs as well.
 */
const OPENTELEMETRY_API = "@opentelemetry/api";

/** The longest `traceparent` a receiver reads (ADR 0132). */
const MAX_TRACEPARENT = 256;

/** The longest `tracestate` a receiver keeps (ADR 0132). */
const MAX_TRACESTATE = 512;

/** The length of a version-00 `traceparent`. */
const TRACEPARENT_LENGTH = 55;

/**
 * The Trace context of one call, `RequestMeta`'s `traceparent` and
 * `tracestate`: its caller's span, as W3C Trace Context writes it.
 */
export interface TraceContext {
  readonly traceparent: string;
  readonly tracestate?: string;
}

/** Reads the calling context's Trace context, `undefined` outside any span. */
export type TraceContextSource = () => TraceContext | undefined;

/** Runs `work` under an incoming request's Trace context, when present and valid. */
export type TraceContextRunner = <T>(
  context: TraceContext | undefined,
  work: () => Promise<T>,
) => Promise<T>;

/** The OpenTelemetry hooks loaded once for a Connection. */
export interface TraceContextHooks {
  readonly source: TraceContextSource;
  readonly run: TraceContextRunner;
}

/** The source of a program without the OpenTelemetry API: no call carries a Trace context. */
const NONE_SOURCE: TraceContextSource = () => undefined;

/** The runner of a program without the OpenTelemetry API: runs `work` unchanged. */
const PASSTHROUGH_RUNNER: TraceContextRunner = (_context, work) => work();

const NONE_HOOKS: TraceContextHooks = {
  source: NONE_SOURCE,
  run: PASSTHROUGH_RUNNER,
};

let loading: Promise<TraceContextHooks> | undefined;

/**
 * Loads the OpenTelemetry API once per process for both outgoing calls and
 * incoming SDK handler requests; falls back cleanly when `@opentelemetry/api`
 * is not installed.
 */
export function traceContextHooks(): Promise<TraceContextHooks> {
  loading ??= import(/* @vite-ignore */ OPENTELEMETRY_API).then(hooksOf, () => NONE_HOOKS);
  return loading;
}

/**
 * The source that reads the program's active OpenTelemetry context, loaded
 * once per process; `NONE` when `@opentelemetry/api` is not installed or is
 * not the API this reads.
 */
export async function traceContextSource(): Promise<TraceContextSource> {
  return (await traceContextHooks()).source;
}

/** The hooks reading and activating contexts on `api`, or no-op hooks when it is not one. */
function hooksOf(api: unknown): TraceContextHooks {
  const contextApi = member(api, "context");
  const active = member(contextApi, "active");
  const withContext = member(contextApi, "with");
  const traceApi = member(api, "trace");
  const getSpanContext = member(traceApi, "getSpanContext");
  const setSpanContext = member(traceApi, "setSpanContext");
  const isSpanContextValid = member(api, "isSpanContextValid");
  const createTraceState = member(api, "createTraceState");
  if (
    typeof active !== "function" ||
    typeof getSpanContext !== "function" ||
    typeof isSpanContextValid !== "function"
  ) {
    return NONE_HOOKS;
  }
  const source: TraceContextSource = () => {
    const spanContext: unknown = Reflect.apply(getSpanContext, undefined, [
      Reflect.apply(active, contextApi, []),
    ]);
    if (
      spanContext === undefined ||
      Reflect.apply(isSpanContextValid, undefined, [spanContext]) !== true
    ) {
      return undefined;
    }
    return traceContextOf(spanContext);
  };
  const run: TraceContextRunner =
    typeof withContext === "function" && typeof setSpanContext === "function"
      ? (received, work) => {
          const parsed =
            received === undefined ? undefined : parseTraceparent(received.traceparent);
          if (parsed === undefined) {
            return work();
          }
          const traceState =
            received?.tracestate !== undefined &&
            received.tracestate.length <= MAX_TRACESTATE &&
            typeof createTraceState === "function"
              ? safeTraceState(createTraceState, received.tracestate)
              : undefined;
          const spanContext = {
            traceId: parsed.traceId,
            spanId: parsed.spanId,
            traceFlags: parsed.traceFlags,
            isRemote: true,
            ...(traceState === undefined ? {} : { traceState }),
          };
          const parent: unknown = Reflect.apply(setSpanContext, traceApi, [
            Reflect.apply(active, contextApi, []),
            spanContext,
          ]);
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `context.with` returns the callback's return value unchanged.
          return Reflect.apply(withContext, contextApi, [parent, work]) as Promise<never>;
        }
      : PASSTHROUGH_RUNNER;
  return { source, run };
}

/** Builds an OpenTelemetry `TraceState`, dropping an unreadable header per ADR 0132. */
function safeTraceState(createTraceState: unknown, header: string): unknown {
  if (typeof createTraceState !== "function") {
    return undefined;
  }
  try {
    return Reflect.apply(createTraceState, undefined, [header]);
  } catch {
    return undefined;
  }
}

/**
 * Extracts `_meta`'s Trace context from an incoming JSON-RPC request's
 * params, returning `undefined` when absent or unusable (ADR 0132).
 */
export function extractTraceContext(params: unknown): TraceContext | undefined {
  if (!isRecord(params)) {
    return undefined;
  }
  const meta = params[META];
  if (!isRecord(meta)) {
    return undefined;
  }
  const traceparent = meta["traceparent"];
  if (typeof traceparent !== "string") {
    return undefined;
  }
  const tracestate = meta["tracestate"];
  return typeof tracestate === "string" ? { traceparent, tracestate } : { traceparent };
}

function isLowerHex(value: string, length: number): boolean {
  return value.length === length && /^[0-9a-f]+$/.test(value);
}

/**
 * Parses a W3C Trace Context `traceparent` header under ADR 0132's rules:
 * lowercase hex, version other than `ff`, non-zero trace and span IDs,
 * length <= 256, and exact length 55 at version `00` (or `-`-delimited
 * extension at a later version).
 */
function parseTraceparent(
  traceparent: string,
): { readonly traceId: string; readonly spanId: string; readonly traceFlags: number } | undefined {
  if (traceparent.length < TRACEPARENT_LENGTH || traceparent.length > MAX_TRACEPARENT) {
    return undefined;
  }
  const head = traceparent.slice(0, TRACEPARENT_LENGTH);
  const rest = traceparent.slice(TRACEPARENT_LENGTH);
  const fields = head.split("-");
  if (fields.length !== 4) {
    return undefined;
  }
  const [version, traceId, spanId, flags] = fields;
  if (
    version === undefined ||
    traceId === undefined ||
    spanId === undefined ||
    flags === undefined
  ) {
    return undefined;
  }
  if (
    !isLowerHex(version, 2) ||
    version === "ff" ||
    !isLowerHex(traceId, 32) ||
    !isLowerHex(spanId, 16) ||
    !isLowerHex(flags, 2) ||
    /^0+$/.test(traceId) ||
    /^0+$/.test(spanId)
  ) {
    return undefined;
  }
  if (version === "00" ? rest.length !== 0 : rest.length !== 0 && !rest.startsWith("-")) {
    return undefined;
  }
  return { traceId, spanId, traceFlags: Number.parseInt(flags, 16) };
}

/** `spanContext`'s `traceparent`, and its `tracestate` when it has one (W3C Trace Context, version `00`). */
function traceContextOf(spanContext: unknown): TraceContext | undefined {
  const traceId = member(spanContext, "traceId");
  const spanId = member(spanContext, "spanId");
  const traceFlags = member(spanContext, "traceFlags");
  if (typeof traceId !== "string" || typeof spanId !== "string" || typeof traceFlags !== "number") {
    return undefined;
  }
  const flags = (traceFlags & 0xff).toString(16).padStart(2, "0");
  const traceparent = `00-${traceId}-${spanId}-${flags}`;
  const traceState = member(spanContext, "traceState");
  const serialize = member(traceState, "serialize");
  const tracestate: unknown =
    typeof serialize === "function" ? Reflect.apply(serialize, traceState, []) : undefined;
  return typeof tracestate === "string" && tracestate !== ""
    ? { traceparent, tracestate }
    : { traceparent };
}

/** `value`'s member `key`, `undefined` when `value` is no object. */
function member(value: unknown, key: string): unknown {
  return (typeof value === "object" || typeof value === "function") && value !== null
    ? Reflect.get(value, key)
    : undefined;
}

/**
 * `params` carrying `context` in its `_meta`, beside what that already
 * holds, such as a raw call's mark; no context leaves `params` as it is.
 */
export function carry(params: unknown, context: TraceContext | undefined): unknown {
  if (context === undefined) {
    return params;
  }
  if (params === undefined || params === null) {
    return { [META]: { ...context } };
  }
  if (typeof params !== "object" || Array.isArray(params)) {
    return params;
  }
  const meta = member(params, META);
  return {
    ...params,
    [META]: typeof meta === "object" && meta !== null ? { ...meta, ...context } : { ...context },
  };
}
