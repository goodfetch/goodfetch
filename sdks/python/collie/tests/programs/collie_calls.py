"""What the integration tests share to call an Attached engine and read its answers.

A Connection opened one of the three ways, the Goodfetch error a call raises,
and the lines the engine wrote on its stderr. TypeScript's `failure` and
`recordingLogger` of `@goodfetch/test-support` are their twins.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
from goodfetch_collie import AsyncCollie, Collie, GoodfetchError

if TYPE_CHECKING:
    from collections.abc import Awaitable, Mapping

    from goodfetch_collie import Limits
    from goodfetch_test_support import Way

CLIENT_LOGGER = "goodfetch.client"
"""The logger the engine's stderr lines and a refused log level go to."""


async def open_collie(
    way: Way, env: Mapping[str, str], limits: Limits | None = None
) -> AsyncCollie:
    """A Connection this way opened on the Attached engine `env` names."""
    return await way.open(
        lambda: AsyncCollie.connect(env=env, limits=limits),
        lambda: Collie.connect(env=env, limits=limits),
    )


def engine_warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
    """Each line the engines wrote on stderr, which a failed play writes."""
    return [
        record.getMessage() for record in caplog.records if record.name == CLIENT_LOGGER
    ]


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises; a call that succeeds fails the test."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value
