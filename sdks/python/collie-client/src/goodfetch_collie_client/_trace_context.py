"""The Trace context each request carries (ADR 0132).

The host program's current OpenTelemetry context, read through
`opentelemetry-api` when the program can import it, so that the program's
trace continues in the engine. The SDK requires no OpenTelemetry package:
without one, no request carries a Trace context and nothing fails. The
context is read as each call is made, in the caller's task; a call of the
sync API, which runs on the Connection's private thread, runs in a copy of
its caller's thread's context (`portal`), so every request it makes
carries that thread's Trace context. Only `traceparent` and `tracestate` are sent,
never Baggage, in the request's `_meta`, which no public type names. The
SDK opens no span of its own. The Connection's own Subscription, shared by
every stream of the events it publishes, belongs to no one caller, so its
request carries none, nor do `hello` and the Limits set as the Connection
opens. Rust's `trace_context.rs` of `goodfetch-collie-client` and
TypeScript's `trace-context.ts` are its twins.
"""

from __future__ import annotations

from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass
from functools import cache
from typing import TYPE_CHECKING, Final, cast

if TYPE_CHECKING:
    from collections.abc import Callable, Generator, Mapping

__all__ = ["TraceContext", "carry", "current", "untraced"]

_META: Final = "_meta"
"""The member of a request's params that holds its envelope (ADR 0132)."""


@dataclass(frozen=True)
class TraceContext:
    """The Trace context of one call: its caller's span, in W3C Trace Context."""

    traceparent: str
    tracestate: str | None = None

    def wire(self) -> dict[str, str]:
        """Its members of `RequestMeta`, a `tracestate` left out when it has none."""
        if self.tracestate is None:
            return {"traceparent": self.traceparent}
        return {"traceparent": self.traceparent, "tracestate": self.tracestate}


_UNTRACED: ContextVar[bool] = ContextVar("goodfetch_untraced", default=False)
"""Whether the calls made now are the Connection's own, which carry none."""


def current() -> TraceContext | None:
    """The Trace context a call made now carries.

    The program's current OpenTelemetry span's; `None` outside any span,
    without `opentelemetry-api`, or for the Connection's own calls, inside
    `untraced`.
    """
    if _UNTRACED.get():
        return None
    read = _reader()
    return None if read is None else read()


@contextmanager
def untraced() -> Generator[None]:
    """Makes the calls made within the Connection's own: they carry no Trace context."""
    token = _UNTRACED.set(True)
    try:
        yield
    finally:
        _UNTRACED.reset(token)


def carry(
    params: Mapping[str, object], context: TraceContext | None
) -> Mapping[str, object]:
    """`params` carrying `context` in its `_meta`, beside what that already holds.

    Such as a raw call's mark; no context leaves the params as they are.
    """
    if context is None:
        return params
    sent = dict(params)
    meta = sent.get(_META)
    # A `_meta` the SDK wrote itself, a raw call's mark: a program's is refused.
    held: Mapping[str, object] = (
        cast("Mapping[str, object]", meta) if isinstance(meta, dict) else {}
    )
    sent[_META] = {**held, **context.wire()}
    return sent


@cache
def _reader() -> Callable[[], TraceContext | None] | None:
    """What reads the current span's Trace context, once per process.

    `None` when `opentelemetry-api` cannot be imported.
    """
    try:
        from opentelemetry import trace
    except ImportError:
        return None

    def read() -> TraceContext | None:
        span = trace.get_current_span().get_span_context()
        if not span.is_valid:
            return None
        traceparent = (
            f"00-{span.trace_id:032x}-{span.span_id:016x}-{span.trace_flags:02x}"
        )
        tracestate = span.trace_state.to_header()
        return TraceContext(traceparent, tracestate or None)

    return read
