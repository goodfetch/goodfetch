"""The Trace context each request carries (ADR 0132).

The host program's current OpenTelemetry context, read through
`opentelemetry-api` when the program can import it, so that the program's
trace continues in the engine; and the reverse for incoming requests from
the engine to an SDK handler, making the engine's Trace context current
around the handler so its spans nest under the engine's. The SDK requires
no OpenTelemetry package: without one, no request carries a Trace context
and nothing fails. The context is read as each call is made, in the
caller's task; a call of the sync API, which runs on the Connection's
private thread, runs in a copy of its caller's thread's context (`portal`),
so every request it makes carries that thread's Trace context. Only
`traceparent` and `tracestate` are sent or read, never Baggage, in the
request's `_meta`, which no public type names. The SDK opens no span of
its own. The Connection's own Subscription, shared by every stream of the
events it publishes, belongs to no one caller, so its request carries none,
nor do `hello` and the Limits set as the Connection opens. Rust's
`trace_context.rs` of `goodfetch-collie-client` and TypeScript's
`trace-context.ts` are its twins.
"""

from __future__ import annotations

from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass
from functools import cache
from typing import TYPE_CHECKING, Final, cast

from goodfetch_collie_client._framing import object_members

if TYPE_CHECKING:
    from collections.abc import Callable, Generator, Mapping

__all__ = [
    "TraceContext",
    "carry",
    "current",
    "extract",
    "untraced",
    "with_trace_context",
]

_META: Final = "_meta"
"""The member of a request's params that holds its envelope (ADR 0132)."""

_MAX_TRACEPARENT: Final = 256
"""The longest `traceparent` a receiver reads (ADR 0132)."""

_MAX_TRACESTATE: Final = 512
"""The longest `tracestate` a receiver keeps (ADR 0132)."""

_TRACEPARENT_LENGTH: Final = 55
"""The length of a version-00 `traceparent`."""

_HEX_DIGITS: Final = frozenset("0123456789abcdef")


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


def extract(params: object) -> TraceContext | None:
    """Extracts `_meta`'s Trace context from an incoming JSON-RPC request's params."""
    members = object_members(params)
    if members is None:
        return None
    meta = object_members(members.get(_META))
    if meta is None:
        return None
    traceparent = meta.get("traceparent")
    if not isinstance(traceparent, str):
        return None
    tracestate = meta.get("tracestate")
    return TraceContext(
        traceparent, tracestate if isinstance(tracestate, str) else None
    )


def _parse_traceparent(traceparent: str) -> tuple[int, int, int] | None:
    """Parses a W3C `traceparent` header under ADR 0132's rules."""
    if len(traceparent) < _TRACEPARENT_LENGTH or len(traceparent) > _MAX_TRACEPARENT:
        return None
    head = traceparent[:_TRACEPARENT_LENGTH]
    rest = traceparent[_TRACEPARENT_LENGTH:]
    parts = head.split("-")
    if len(parts) != 4:
        return None
    version, trace_id_hex, span_id_hex, flags_hex = parts
    if (
        len(version) != 2
        or version == "ff"
        or len(trace_id_hex) != 32
        or len(span_id_hex) != 16
        or len(flags_hex) != 2
        or not all(
            all(ch in _HEX_DIGITS for ch in field)
            for field in (version, trace_id_hex, span_id_hex, flags_hex)
        )
    ):
        return None
    if version == "00":
        if rest:
            return None
    elif rest and not rest.startswith("-"):
        return None
    trace_id = int(trace_id_hex, 16)
    span_id = int(span_id_hex, 16)
    if trace_id == 0 or span_id == 0:
        return None
    return trace_id, span_id, int(flags_hex, 16)


@contextmanager
def with_trace_context(meta: TraceContext | None) -> Generator[None]:
    """Makes `meta` the current OpenTelemetry context while the block runs (ADR 0132).

    Without `opentelemetry-api`, or when `meta` is absent or invalid, yields
    unchanged without failing.
    """
    if meta is None:
        yield
        return
    parsed = _parse_traceparent(meta.traceparent)
    if parsed is None:
        yield
        return
    attacher = _attacher()
    if attacher is None:
        yield
        return
    tracestate = (
        meta.tracestate
        if meta.tracestate is not None and len(meta.tracestate) <= _MAX_TRACESTATE
        else None
    )
    detach = attacher(*parsed, tracestate)
    try:
        yield
    finally:
        detach()


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


@cache
def _attacher() -> Callable[[int, int, int, str | None], Callable[[], None]] | None:
    """Attaches a remote W3C Trace context as current, returning a detach callback."""
    try:
        from opentelemetry import context as otel_context
        from opentelemetry import trace
        from opentelemetry.trace import (
            NonRecordingSpan,
            SpanContext,
            TraceFlags,
            TraceState,
        )
    except ImportError:
        return None

    def attach(
        trace_id: int, span_id: int, flags: int, tracestate: str | None
    ) -> Callable[[], None]:
        state = TraceState.from_header([tracestate]) if tracestate else TraceState()
        span = NonRecordingSpan(
            SpanContext(
                trace_id=trace_id,
                span_id=span_id,
                is_remote=True,
                trace_flags=TraceFlags(flags),
                trace_state=state,
            )
        )
        token = otel_context.attach(trace.set_span_in_context(span))
        return lambda: otel_context.detach(token)

    return attach
