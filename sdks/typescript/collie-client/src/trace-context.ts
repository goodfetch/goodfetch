// The Trace context each request carries (ADR 0132): the host program's
// active OpenTelemetry context, read through `@opentelemetry/api` when that
// optional peer dependency is installed, so that the program's trace
// continues in the engine. Without it, no request carries one and nothing
// fails. The package is loaded once, before a Connection's `hello`, and
// read as each call is made, in the caller's context. Only `traceparent`
// and `tracestate` are sent, never Baggage, in the request's `_meta`, which
// no public type names. The SDK opens no span of its own. The Connection's
// own Subscription, shared by every stream of the events it publishes,
// belongs to no one caller, so its request carries none.
/** The member of a request's params that holds its envelope (ADR 0132). */
const META = "_meta";

/**
 * The OpenTelemetry API's package, named in a value so that neither the
 * type checker nor a bundler resolves it: a program that does not install
 * it builds and runs as well.
 */
const OPENTELEMETRY_API = "@opentelemetry/api";

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

/** The source of a program without the OpenTelemetry API: no call carries a Trace context. */
const NONE: TraceContextSource = () => undefined;

let loading: Promise<TraceContextSource> | undefined;

/**
 * The source that reads the program's active OpenTelemetry context, loaded
 * once per process; `NONE` when `@opentelemetry/api` is not installed or is
 * not the API this reads.
 */
export function traceContextSource(): Promise<TraceContextSource> {
  loading ??= import(/* @vite-ignore */ OPENTELEMETRY_API).then(sourceOf, () => NONE);
  return loading;
}

/** The source reading `api`, the OpenTelemetry API's module, or `NONE` when it is not one. */
function sourceOf(api: unknown): TraceContextSource {
  const contextApi = member(api, "context");
  const active = member(contextApi, "active");
  const getSpanContext = member(member(api, "trace"), "getSpanContext");
  const isSpanContextValid = member(api, "isSpanContextValid");
  if (
    typeof active !== "function" ||
    typeof getSpanContext !== "function" ||
    typeof isSpanContextValid !== "function"
  ) {
    return NONE;
  }
  return () => {
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
