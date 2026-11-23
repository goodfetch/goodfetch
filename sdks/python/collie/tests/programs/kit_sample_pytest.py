"""The testing kit's sample test under pytest with AnyIO's plugin (ADR 0105).

A program's own test file, as its author writes it: no plugin of
Goodfetch's, the guide's three-line fixture opening `AsyncTestEngine`, the
async test run on asyncio and on Trio and the same test through the sync
`TestEngine`. The engine comes from the process's environment, as a test
run sets it. `test_test_engine.py` runs it in a folder of its own, so no
configuration but pytest's defaults applies, with pytest's collection
warnings as errors: `TestEngine`, imported here, is never collected.
TypeScript's `tests/fixtures/kit-sample/sample.ts` and Rust's
`tests/it/sdk/test_engine.rs` are its twins.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
from goodfetch_collie.testing import AsyncTestEngine, TestEngine

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from goodfetch_collie import AsyncCollie, Collie

pytestmark = pytest.mark.anyio


@pytest.fixture(params=["asyncio", "trio"])
def anyio_backend(request: pytest.FixtureRequest) -> object:
    """Each async test runs on asyncio and on Trio."""
    return request.param


@pytest.fixture
async def engine() -> AsyncIterator[AsyncTestEngine]:
    """A test engine for one test, closed after it."""
    async with AsyncTestEngine.start() as engine:
        yield engine


async def test_a_run_finishes_on_the_test_engine(engine: AsyncTestEngine) -> None:
    """The code under test takes the Product's own connection."""
    connection: AsyncCollie = engine.connection

    report = await (await connection.start()).finish()

    assert report.lifecycle == "finished"


def test_a_run_finishes_on_the_sync_test_engine() -> None:
    """The same test through the sync API."""
    with TestEngine.start() as engine:
        connection: Collie = engine.connection

        report = connection.start().finish()

    assert report.lifecycle == "finished"
