"""The pytest plugin registered through `pytest11` (ADR 0062).

It gives the fixtures that reach `goodfetch-test-env`'s setup:

- `test_env`, the test's `TestEnv`;
- `env`, the environment map an SDK takes as `env=`, naming the engine of
  the test's `product`;
- `product`, Collie unless the test overrides it with Foxhound (ADR 0094).
"""

import os
from collections.abc import Iterator, Mapping
from typing import cast

import pytest

from goodfetch_test_support.environment import Product, TestEnv


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


@pytest.fixture
def product() -> Product:
    """The Product whose executable the test runs: Collie unless overridden."""
    return Product.COLLIE


@pytest.fixture
def env(test_env: TestEnv, product: Product) -> Mapping[str, str]:
    """The test's cleared environment map with its Product's engine path.

    It holds `GOODFETCH_HOME` and the engine-path variable of `product`,
    `GOODFETCH_COLLIE_ENGINE_PATH` or `GOODFETCH_FOXHOUND_ENGINE_PATH`
    (ADR 0090), pointing at that Product's executable from the test tool
    manifest.
    """
    engine = test_env.tools.path(product.tool)
    return {**test_env.env, product.engine_path_variable: str(engine)}


PATCHING_FIXTURES = frozenset({"monkeypatch", "mocker"})
"""Fixtures that patch globals or mock owned code, which no test may reach."""


def pytest_collection_modifyitems(
    session: pytest.Session, items: list[pytest.Item]
) -> None:
    """Fails the collection of each test whose fixture closure patches.

    The closure holds every fixture the test requests, directly or through
    another fixture, so a third-party fixture built on `monkeypatch` is
    refused too (ADR 0062).
    """
    refused: list[pytest.Item] = []
    for item in items:
        patching = sorted(PATCHING_FIXTURES.intersection(_fixture_closure(item)))
        if patching:
            refused.append(item)
            report = pytest.CollectReport(
                item.nodeid,
                "failed",
                f"{item.nodeid} reaches {', '.join(patching)}, directly or through "
                "another fixture: pass the value through an option instead (ADR 0062)",
                [],
            )
            session.ihook.pytest_collectreport(report=report)
    items[:] = [item for item in items if item not in refused]


def _fixture_closure(item: pytest.Item) -> list[str]:
    """Every fixture `item` reaches; an item that is no function reaches none."""
    names: object = getattr(item, "fixturenames", [])
    return cast("list[str]", names)
