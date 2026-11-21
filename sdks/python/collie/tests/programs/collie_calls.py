"""What the integration tests share to call an Attached engine and read its answers.

A Connection opened one of the three ways, the Goodfetch error a call raises,
and the lines the engine wrote on its stderr. TypeScript's `failure` and
`recordingLogger` of `@goodfetch/test-support` are their twins.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import pytest
from goodfetch_collie import AsyncCollie, Collie, GoodfetchError
from goodfetch_collie_client.handle import NotGiven, not_given

if TYPE_CHECKING:
    from collections.abc import Awaitable, Mapping
    from datetime import timedelta

    from goodfetch_collie import EngineChoice, Limits
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
