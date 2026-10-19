"""The pytest plugin registered through `pytest11` (ADR 0062).

It gives the fixtures that reach `goodfetch-test-env`'s setup.
"""

import os
from collections.abc import Iterator
from typing import cast

import pytest

from goodfetch_test_support.environment import TestEnv


@pytest.fixture
def test_env(request: pytest.FixtureRequest) -> Iterator[TestEnv]:
    """The test's setup from `goodfetch-test-env`, torn down after the test.

    The teardown fails the test when a process it started is still alive.
    """
    # pytest leaves `FixtureRequest.node` unannotated.
    item = cast("pytest.Item", request.node)  # pyright: ignore[reportUnknownMemberType]
    env = TestEnv.open(item.nodeid, os.environ)
    yield env
    env.close()
