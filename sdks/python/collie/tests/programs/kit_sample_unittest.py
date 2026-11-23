"""The testing kit's sample test under `unittest` (ADR 0105).

A program's own test module, as its author writes it: the async test opens
`AsyncTestEngine` on `IsolatedAsyncioTestCase`'s loop, and the same test
runs through the sync `TestEngine`. The engine comes from the process's
environment, as a test run sets it. `test_test_engine.py` runs it with
`python -m unittest`; `kit_sample_pytest.py` is its pytest twin.
"""

from __future__ import annotations

import unittest
from typing import TYPE_CHECKING

from goodfetch_collie.testing import AsyncTestEngine, TestEngine

if TYPE_CHECKING:
    from goodfetch_collie import AsyncCollie, Collie


class ARunFinishesOnTheTestEngine(unittest.IsolatedAsyncioTestCase):
    """The code under test takes the Product's own connection."""

    async def asyncSetUp(self) -> None:
        """A test engine for each test, closed after it."""
        self.engine = await self.enterAsyncContext(AsyncTestEngine.start())

    async def test_a_run_finishes(self) -> None:
        """The Run the test starts finishes."""
        connection: AsyncCollie = self.engine.connection

        report = await (await connection.start()).finish()

        self.assertEqual(report.lifecycle, "finished")


class ARunFinishesOnTheSyncTestEngine(unittest.TestCase):
    """The same test through the sync API."""

    def setUp(self) -> None:
        """A test engine for each test, closed after it."""
        self.engine = self.enterContext(TestEngine.start())

    def test_a_run_finishes(self) -> None:
        """The Run the test starts finishes."""
        connection: Collie = self.engine.connection

        report = connection.start().finish()

        self.assertEqual(report.lifecycle, "finished")


if __name__ == "__main__":
    unittest.main()
