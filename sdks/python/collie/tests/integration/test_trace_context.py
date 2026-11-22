"""The program's Trace context in each request (ADR 0132), on the Scripted engine.

The `trace-context` client conformance case, three ways: async on asyncio,
async on Trio and through the sync `Collie`. With `opentelemetry-api`
installed, a call made in the program's current span sends that span's
`traceparent`, and its `tracestate`, in its `_meta`, a raw call's beside
its raw mark; a call made outside any span sends no `_meta`, nor does
`hello`, the Connection's own, though opened in the span. The sync API
sends the context of the thread that calls it, not the private thread's:
two threads in two spans share one `Collie`. The Scripted engine fails on
params it does not expect and on a step left unused, so a play with no
warning, or a Daemon's clean exit, says each call carried exactly that.
TypeScript's `trace-context.test.ts` and Rust's `trace_context.rs` are its
twins.
"""

import threading
from collections.abc import Awaitable
from concurrent.futures import ThreadPoolExecutor

import pytest
from collie_calls import engine_warnings, exit_code, open_collie
from goodfetch_collie import AsyncCollie, Collie
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct
from opentelemetry import trace
from opentelemetry.trace import NonRecordingSpan, SpanContext, TraceFlags, TraceState

COVERS = pytest.mark.covers("story:11-138")

PROGRAM_SPAN = NonRecordingSpan(
    SpanContext(
        trace_id=0x0AF7651916CD43DD8448EB211C80319C,
        span_id=0x1A2B3C4D5E6F7081,
        is_remote=False,
        trace_flags=TraceFlags(TraceFlags.SAMPLED),
        trace_state=TraceState([("vendor", "value")]),
    )
)
"""The program's span, whose context has a `tracestate`, as the scripts expect it."""

OTHER_SPAN = NonRecordingSpan(
    SpanContext(
        trace_id=0x4BF92F3577B34DA6A3CE929D0E0E4736,
        span_id=0xB7AD6B7169203331,
        is_remote=False,
        trace_flags=TraceFlags(TraceFlags.SAMPLED),
    )
)
"""A second thread's span, in another trace, with no `tracestate`."""


async def play_the_case(opening: Awaitable[AsyncCollie]) -> None:
    """The case: the opening and three calls in the program's span, two outside."""
    with trace.use_span(PROGRAM_SPAN):
        collie = await opening
        await collie.explain_settings()
        await collie.call("run.list", {"lifecycle": "running"})
        run = await collie.start()
    await collie.explain_settings()
    await run.finish()
    await collie.close()


@COVERS
@pytest.mark.client_case(id="trace-context", mode="attached")
@three_ways
async def test_a_call_in_the_programs_span_carries_its_trace_context(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """In the span, each call carries its context; outside it, none does."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/trace-context.json"
    )

    await play_the_case(open_collie(way, env))

    with client_case.check(
        "the engine read each call's `_meta` as its script holds it, to the end"
    ):
        assert engine_warnings(caplog) == []


@COVERS
@pytest.mark.client_case(id="trace-context", mode="daemon")
@three_ways
async def test_a_call_in_the_programs_span_carries_its_trace_context_to_a_daemon(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
) -> None:
    """The same calls through a Daemon carry the same Trace contexts."""
    daemon = start_scripted_daemon(
        test_env, "sdk-python/connect/trace-context-through-a-daemon.json", "socket"
    )

    await play_the_case(open_collie(way, test_env.env, engine="daemon"))
    code = await exit_code(daemon.wait)

    with client_case.check(
        "the Daemon read each call's `_meta` as its script holds it, to the end"
    ):
        assert code == 0


@COVERS
def test_each_thread_of_the_sync_api_sends_its_own_trace_context(
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Two threads, both in their spans before either calls, each send their own."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/trace-context-from-two-threads.json"
    )
    first_in_span = threading.Event()
    second_in_span = threading.Event()
    first_called = threading.Event()
    collie = Collie.connect(env=env)

    def first() -> None:
        with trace.use_span(PROGRAM_SPAN):
            first_in_span.set()
            second_in_span.wait()
            try:
                collie.explain_settings()
            finally:
                first_called.set()

    def second() -> None:
        with trace.use_span(OTHER_SPAN):
            second_in_span.set()
            first_in_span.wait()
            first_called.wait()
            collie.explain_settings()

    try:
        with ThreadPoolExecutor(max_workers=2) as threads:
            calls = [threads.submit(first), threads.submit(second)]
            for call in calls:
                call.result()
    finally:
        collie.close()

    assert engine_warnings(caplog) == []
