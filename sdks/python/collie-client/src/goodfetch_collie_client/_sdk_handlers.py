"""SDK handlers for Collie's Plugin kinds (ADRs 0033, 0104 and 0132).

Serves incoming Plugin-kind requests from the engine to the Connection's
registered handlers: async handlers run in tasks spawned beside the loop
(cancelled on `$/cancel`), and sync handlers run on worker threads via
`anyio.to_thread.run_sync(..., abandon_on_cancel=True)` (discarding any
late answer on `$/cancel`). A `GoodfetchError` raised by a handler keeps
its code; any other exception becomes the kind's `<kind>_handler_failed`
naming the handler and carrying type, message and stack. Each handler runs
under the incoming request's `_meta` Trace context.
"""

from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from typing import TYPE_CHECKING, TypeAlias

import anyio
import anyio.to_thread

from goodfetch_collie_client._trace_context import (
    TraceContext,
    extract,
    with_trace_context,
)
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.errors import (
    ExtractHandlerFailedError,
    ExtractorUnavailableError,
)
from goodfetch_collie_client.generated.models import (
    ExtractContentParams,
    ExtractContentResult,
)
from goodfetch_collie_client.generated.plugins import ExtractHandler
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.listeners import spawner

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client._rpc import ServedRequest

AsyncExtractor: TypeAlias = (
    ExtractHandler | Callable[[ExtractContentParams], Awaitable[ExtractContentResult]]
)
"""An async Extractor handler: an `ExtractHandler` subclass or coroutine function."""

SyncExtractor: TypeAlias = Callable[[ExtractContentParams], ExtractContentResult]
"""A sync Extractor SDK handler: a plain function called on a worker thread."""


def declared_handlers(
    extractors: Mapping[str, object] | None,
) -> dict[str, list[str]] | None:
    """The `hello.handlers` map declaring registered SDK handler names by kind."""
    if not extractors:
        return None
    return {"extract": list(extractors)}


def serve_async_extractors(
    extractors: Mapping[str, AsyncExtractor],
) -> Callable[[ServedRequest], None]:
    """Builds a `ClientConnection.serve` callback for async Extractor handlers."""
    registered = dict(extractors)

    def handle(request: ServedRequest) -> None:
        meta = extract(request.params)

        async def work() -> dict[str, object]:
            params = result_of(ExtractContentParams, request.params)
            handler_name = params.handler or ""
            handler = registered.get(handler_name)
            if handler is None:
                raise ExtractorUnavailableError(
                    f"no Extractor SDK handler named `{handler_name}` is registered "
                    "on this Connection",
                    details={"extractor": handler_name},
                )
            call_params = ExtractContentParams(url=params.url, html=params.html)
            with with_trace_context(meta):
                try:
                    if isinstance(handler, ExtractHandler):
                        answer = await handler.content(call_params)
                    else:
                        answer = await handler(call_params)
                except GoodfetchError:
                    raise
                except Exception as error:
                    raise _extract_handler_failed(
                        handler_name, params.url, error
                    ) from error
            return answer.model_dump(mode="json", by_alias=True, exclude_none=True)

        spawn = spawner()
        spawn(lambda: request.serve(work))

    return handle


def serve_sync_extractors(
    extractors: Mapping[str, SyncExtractor],
) -> Callable[[ServedRequest], None]:
    """Builds a `ClientConnection.serve` callback for sync Extractor handlers.

    Each call runs on a worker thread with `abandon_on_cancel=True`, so a
    `$/cancel` from the engine immediately stops waiting and discards any
    late answer (ADR 0104).
    """
    registered = dict(extractors)

    def handle(request: ServedRequest) -> None:
        meta = extract(request.params)

        async def work() -> dict[str, object]:
            params = result_of(ExtractContentParams, request.params)
            handler_name = params.handler or ""
            handler = registered.get(handler_name)
            if handler is None:
                raise ExtractorUnavailableError(
                    f"no Extractor SDK handler named `{handler_name}` is registered "
                    "on this Connection",
                    details={"extractor": handler_name},
                )
            call_params = ExtractContentParams(url=params.url, html=params.html)
            answer = await anyio.to_thread.run_sync(
                _invoke_sync,
                handler,
                handler_name,
                call_params,
                meta,
                abandon_on_cancel=True,
            )
            return answer.model_dump(mode="json", by_alias=True, exclude_none=True)

        spawn = spawner()
        spawn(lambda: request.serve(work))

    return handle


def _invoke_sync(
    handler: SyncExtractor,
    handler_name: str,
    params: ExtractContentParams,
    meta: TraceContext | None,
) -> ExtractContentResult:
    """Runs a sync Extractor handler under `meta`'s Trace context on a worker thread."""
    with with_trace_context(meta):
        try:
            return handler(params)
        except GoodfetchError:
            raise
        except Exception as error:
            raise _extract_handler_failed(handler_name, params.url, error) from error


def _extract_handler_failed(
    handler_name: str, url: str, error: Exception
) -> ExtractHandlerFailedError:
    """Wraps a handler's own exception in `extract_handler_failed` (ADR 0104)."""
    exc_type = type(error).__name__
    exc_message = str(error)
    exc_stack = traceback.format_exc()
    return ExtractHandlerFailedError(
        f"Extractor SDK handler `{handler_name}` failed with {exc_type}: {exc_message}",
        details={
            "extractor": handler_name,
            "handler": handler_name,
            "url": url,
            "exception_type": exc_type,
            "exception_message": exc_message,
            "exception_stack": exc_stack,
        },
    )
