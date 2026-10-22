"""The pytest plugin registered through `pytest11` (ADR 0062).

It gives the fixtures that reach `goodfetch-test-env`'s setup:

- `test_env`, the test's `TestEnv`;
- `env`, the environment map an SDK takes as `env=`, naming the engine of
  the test's `product`;
- `product`, Collie unless the test overrides it with Foxhound (ADR 0094);
- `proving_ground`, the Proving ground started on `proving_ground_seed`,
  which a test overrides with its seed (ADR 0063);
- `daemon`, the Daemon of the test's home, run by `product`'s executable,
  which a test asks for by overriding `with_daemon` to return `True`.
"""

import json
import os
from collections.abc import Generator, Iterator, Mapping
from pathlib import Path
from typing import cast

import pytest

from goodfetch_test_support.daemon import Daemon, DaemonNotRequested
from goodfetch_test_support.environment import Product, TestEnv
from goodfetch_test_support.proving_ground import ProvingGround, ProvingGroundUnseeded

_FAILED = pytest.StashKey[bool]()
"""Set on a test's item when its call failed, so its setup keeps the folder."""


@pytest.hookimpl(wrapper=True)
def pytest_runtest_makereport(
    item: pytest.Item, call: pytest.CallInfo[None]
) -> Generator[None, pytest.TestReport, pytest.TestReport]:
    """Records whether a test's call failed, for its `test_env` teardown."""
    del call
    report = yield
    if report.when == "call" and report.failed:
        item.stash[_FAILED] = True
    return report


@pytest.fixture
def proving_ground_seed() -> int | None:
    """The seed of the test's Proving ground: none unless the test overrides it."""
    return None


@pytest.fixture
def with_daemon() -> bool:
    """Whether the test's setup starts a Daemon: not unless the test overrides it."""
    return False


@pytest.fixture
def test_env(
    request: pytest.FixtureRequest,
    proving_ground_seed: int | None,
    with_daemon: bool,
    product: Product,
) -> Iterator[TestEnv]:
    """The test's setup from `goodfetch-test-env`, torn down after the test.

    It starts a Proving ground when `proving_ground_seed` names a seed, and
    the Daemon of `product`'s executable in the test's home when
    `with_daemon` is `True`. The teardown stops the Daemon and fails the
    test when a process it started is still alive; a failed test's folder
    stays for its evidence, as a Rust test's does.
    """
    # pytest leaves `FixtureRequest.node` unannotated.
    item = cast("pytest.Item", request.node)  # pyright: ignore[reportUnknownMemberType]
    env = TestEnv.open(
        item.nodeid,
        os.environ,
        proving_ground_seed=proving_ground_seed,
        daemon=product if with_daemon else None,
    )
    yield env
    env.close(failed=item.stash.get(_FAILED, False))


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


@pytest.fixture
def proving_ground(test_env: TestEnv) -> ProvingGround:
    """The Proving ground started on `proving_ground_seed`, stopped at teardown.

    Raises:
        ProvingGroundUnseeded: The test overrides no `proving_ground_seed`.
    """
    if test_env.proving_ground is None:
        message = (
            "the test takes `proving_ground` but sets no seed: override the "
            "`proving_ground_seed` fixture"
        )
        raise ProvingGroundUnseeded(message)
    return test_env.proving_ground


@pytest.fixture
def daemon(test_env: TestEnv) -> Daemon:
    """The Daemon of the test's home, stopped at teardown.

    Raises:
        DaemonNotRequested: The test does not override `with_daemon` to `True`.
    """
    if test_env.daemon is None:
        message = (
            "the test takes `daemon` but asks for none: override the "
            "`with_daemon` fixture to return `True`"
        )
        raise DaemonNotRequested(message)
    return test_env.daemon


LISTING_OPTION = "--marker-listing"
"""Where `--collect-only` writes each test's `journey` and `covers` markers."""


def pytest_addoption(parser: pytest.Parser) -> None:
    """Adds `--marker-listing`."""
    parser.getgroup("goodfetch").addoption(
        LISTING_OPTION,
        metavar="PATH",
        help="write each collected test's journey and covers values to PATH as JSON",
    )


def pytest_configure(config: pytest.Config) -> None:
    """Registers the `journey` and `covers` markers, which `strict_markers` checks."""
    config.addinivalue_line(
        "markers",
        "journey(id, surface): the Journey variant the test runs, as "
        'journey(id="J-002", surface="sdk-python"), which -m selects (ADR 0063)',
    )
    config.addinivalue_line(
        "markers",
        "covers(*features): the Features the test proves, as story:3-18 (ADR 0072)",
    )


def pytest_collection_finish(session: pytest.Session) -> None:
    """Writes the marker listing that the Feature scanner and the Journey check read.

    It lists every collected test by node ID, with the keyword arguments of
    each of its `journey` markers and the arguments of its `covers` markers,
    its own nearest first. An xdist worker never writes it; `--collect-only`
    runs no worker.
    """
    path = cast("str | None", session.config.getoption(LISTING_OPTION))
    if path is None or hasattr(session.config, "workerinput"):
        return
    tests = [
        {
            "id": item.nodeid,
            "journey": [dict(marker.kwargs) for marker in item.iter_markers("journey")],
            "covers": _marker_values(item, "covers"),
        }
        for item in sorted(session.items, key=lambda item: item.nodeid)
    ]
    Path(path).write_text(json.dumps({"tests": tests}, indent=2) + "\n", "utf-8")


def _marker_values(item: pytest.Item, name: str) -> list[object]:
    return [value for marker in item.iter_markers(name) for value in marker.args]


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
