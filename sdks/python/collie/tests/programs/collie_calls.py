"""What the integration tests share to call an Attached engine and read its answers.

A Connection opened one of the three ways, a page's locator and a
response whatever the way, the Goodfetch error a call raises, the lines
the engine wrote on its stderr, and a process's exit code waited for off
the event loop.
TypeScript's `failure` and `recordingLogger` of `@goodfetch/test-support`
are their twins.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import anyio.to_thread
import pytest
from goodfetch_collie import AsyncCollie, Collie, GoodfetchError
from goodfetch_collie_client.handle import NotGiven, not_given

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping
    from datetime import timedelta

    from goodfetch_collie import EngineChoice, Limits
    from goodfetch_collie_client.generated.handles import AsyncLocator, AsyncPage
    from goodfetch_collie_client.response import AsyncResponse
    from goodfetch_test_support import Way

CLIENT_LOGGER = "goodfetch.client"
"""The logger of the engine reached, its stderr lines and a refused log level."""


async def open_collie(
    way: Way,
    env: Mapping[str, str],
    limits: Limits | None = None,
    connect_timeout: timedelta | NotGiven | None = not_given,
    engine: EngineChoice = "auto",
) -> AsyncCollie:
    """A Connection this way opened on the engine `engine` and `env` name."""
    return await way.open(
        lambda: AsyncCollie.connect(
            env=env, engine=engine, limits=limits, timeout=connect_timeout
        ),
        lambda: Collie.connect(
            env=env, engine=engine, limits=limits, timeout=connect_timeout
        ),
    )


def locator_on(way: Way, page: AsyncPage, selector: str) -> AsyncLocator:
    """`page`'s locator for `selector`, its methods async whatever the way.

    A builder is a plain method in both APIs (ADR 0099), so the sync way's
    proxy hands the sync `Locator` itself, which `way.api` wraps.
    """
    built = page.locator(selector)
    return way.api(built, built)


def response_on(way: Way, response: AsyncResponse) -> AsyncResponse:
    """`response`, its body reads async whatever the way.

    A response has no `close`, so the sync way's proxy hands the sync
    `Response` itself, which `way.api` wraps; its head reads are properties,
    which the wrapper hands as they are.
    """
    return way.api(response, response)


def engine_warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
    """Each line the engines wrote on stderr, which a failed play writes.

    The SDK logs them at warning; the engine `connect()` reached, at info,
    is not one.
    """
    return [
        record.getMessage()
        for record in caplog.records
        if record.name == CLIENT_LOGGER and record.levelno >= logging.WARNING
    ]


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises; a call that succeeds fails the test."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


async def exit_code(wait: Callable[[], int]) -> int:
    """The exit code `wait`, a process's blocking wait, returns, off the event loop."""
    return await anyio.to_thread.run_sync(wait)
