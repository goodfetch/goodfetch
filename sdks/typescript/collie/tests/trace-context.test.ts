// The program's Trace context in each request (ADR 0132), the
// `trace-context` client conformance case: with `@opentelemetry/api`
// installed, a call made in the program's active span sends that span's
// `traceparent`, and its `tracestate`, in its `_meta`, a raw call's beside
// its raw mark; a call made outside any span sends no `_meta`, nor does
// `hello`, the Connection's own, though opened in the span. The Scripted
// engine fails on params it does not expect and on a step left unused, so a
// play with no warning, or a Daemon's clean exit, says each call carried
// exactly that. Rust's `trace_context.rs` and Python's
// `test_trace_context.py` are its twins.
import { Collie } from "@goodfetch/collie";
import {
  recordingLogger,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { context, trace, TraceFlags, type SpanContext, type TraceState } from "@opentelemetry/api";
import { AsyncLocalStorageContextManager } from "@opentelemetry/context-async-hooks";
import { afterAll, beforeAll, describe, expect } from "vitest";

/**
 * The `tracestate` the program's span holds, as a propagator read it from
 * the program's own caller: `@opentelemetry/core` implements it, which the
 * API alone does not need.
 */
const TRACE_STATE: TraceState = {
  set: () => TRACE_STATE,
  unset: () => TRACE_STATE,
  get: (key) => (key === "vendor" ? "value" : undefined),
  serialize: () => "vendor=value",
};

/** The program's span, whose context has a `tracestate`, as the scripts expect it. */
const PROGRAM_SPAN: SpanContext = {
  traceId: "0af7651916cd43dd8448eb211c80319c",
  spanId: "1a2b3c4d5e6f7081",
  traceFlags: TraceFlags.SAMPLED,
  traceState: TRACE_STATE,
};

/** `calls`, run with the program's span active, as an instrumented function runs its body. */
function inProgramSpan<T>(calls: () => Promise<T>): Promise<T> {
  return context.with(trace.setSpanContext(context.active(), PROGRAM_SPAN), calls);
}

/** The case: inside the program's span, the opening and three calls; two outside any. */
async function playTheCase(open: () => Promise<Collie>): Promise<void> {
  const { collie, run } = await inProgramSpan(async () => {
    const opened = await open();
    await opened.explainSettings();
    await opened.call("run.list", { lifecycle: "running" });
    return { collie: opened, run: await opened.start() };
  });
  await collie.explainSettings();
  await run.finish();
  await collie.close();
}

// A program traced with OpenTelemetry registers a context manager, which
// carries the active context across its awaits.
beforeAll(() => {
  context.setGlobalContextManager(new AsyncLocalStorageContextManager().enable());
});

afterAll(() => {
  context.disable();
});

describe("the program's Trace context on the Scripted engine", () => {
  test(
    "a call in the program's span carries its Trace context, and one outside it none, on the Attached engine",
    { tags: ["client_case/trace-context", "attached", "story:11-138"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/trace-context.json");
      const { logger, warned } = recordingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await playTheCase(() => Collie.connect({ env, logger }));

      await clientCase.check(
        "the engine read each call's `_meta` as the script holds it and played to its end",
        () => expect(warned).toEqual([]),
      );
    },
  );

  test(
    "a call in the program's span carries its Trace context, and one outside it none, through a Daemon",
    { tags: ["client_case/trace-context", "daemon", "story:11-138"] },
    async ({ testEnv, processes, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/trace-context-through-a-daemon.json",
        "socket",
      );

      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await playTheCase(() => Collie.connect({ env }));
      const exit = await daemon.exit;

      await clientCase.check(
        "the Daemon read each call's `_meta` as the script holds it and played to its end",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
