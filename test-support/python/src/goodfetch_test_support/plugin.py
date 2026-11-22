"""The pytest plugin registered through `pytest11` (ADR 0062).

It gives the fixtures that reach `goodfetch-test-env`'s setup:

- `test_env`, the test's `TestEnv`;
- `env`, the environment map an SDK takes as `env=`, naming the engine of
  the test's `product`;
- `product`, Collie unless the test overrides it with Foxhound (ADR 0094);
- `proving_ground`, the Proving ground started on `proving_ground_seed`,
  which a test overrides with its seed (ADR 0063);
- `daemon`, the Daemon of the test's home, run by `product`'s executable,
  which a test asks for by overriding `with_daemon` to return `True`;
- `with_browser`, which a test whose engine opens a page overrides to
  return `True`: the home's Browser cache laid out, and the Proving
  ground's hosts reachable;
- `anyio_backend`, over asyncio and trio, named: a session where trio does
  not import fails rather than drop it (ADR 0069);
- `client_case`, the `ClientCaseRecord` of the Client conformance case a
  test carries as `client_case(id="<case>", mode="attached")`, whose
  checks and evidence go into the case log `--case-log` names (ADR 0069).

It fails a session started with `PYTHON_GIL=0` while the GIL is on, and runs
each `sync_api` test in several threads at once through pytest-run-parallel
(ADR 0069).
"""

import importlib
import json
import os
import sys
from collections.abc import Generator, Iterator, Mapping
from pathlib import Path
from typing import cast

import pytest

from goodfetch_test_support.case_log import (
    CLIENT_CASE_MODES,
    CaseListInvalid,
    ClientCaseRecord,
    ClientCaseUntagged,
    case_log_lines,
    read_client_cases,
)
from goodfetch_test_support.daemon import Daemon, DaemonNotRequested
from goodfetch_test_support.environment import Product, TestEnv
from goodfetch_test_support.mutation_children import MutationChildren
from goodfetch_test_support.proving_ground import ProvingGround, ProvingGroundUnseeded
from goodfetch_test_support.ways import ASYNC_BACKENDS

_FAILED = pytest.StashKey[bool]()
"""Set on a test's item when its call failed, so its setup keeps the folder."""
_REPORTS = pytest.StashKey[list[pytest.TestReport]]()
"""A test's reports, setup, call and teardown, for its case log entries."""
_RECORD = pytest.StashKey[ClientCaseRecord]()
"""What a test recorded of its Client conformance case through `client_case`."""
_CASE_LOG = pytest.StashKey[Path | None]()
"""The case log the run writes, if any."""
_CASES = pytest.StashKey[dict[str, str] | None]()
"""Each case of the run's case list with its capability, if it names one."""
_CHILDREN = pytest.StashKey[MutationChildren]()
"""The mutmut run's reach into the programs tests start, under `--mutation-children`."""
_ENVIRON = pytest.StashKey[dict[str, str | None]]()
"""The runner's variables that `--mutation-children` set, as they were, restored at
the end; `None` for one the runner did not have."""

CASE_MARKER = "client_case"
"""The marker that carries a test's Client conformance case and mode."""


@pytest.hookimpl(wrapper=True)
def pytest_runtest_makereport(
    item: pytest.Item, call: pytest.CallInfo[None]
) -> Generator[None, pytest.TestReport, pytest.TestReport]:
    """Records whether a test's call failed, for its `test_env` teardown.

    It keeps each report of a test that carries a `client_case` marker, for
    its case log entries.
    """
    del call
    report = yield
    if report.when == "call" and report.failed:
        item.stash[_FAILED] = True
    if item.get_closest_marker(CASE_MARKER) is not None:
        item.stash.setdefault(_REPORTS, []).append(report)
    return report


@pytest.hookimpl(wrapper=True)
def pytest_runtest_protocol(
    item: pytest.Item, nextitem: pytest.Item | None
) -> Generator[None, object, object]:
    """Writes a test's case log entries once it has run, setup to teardown.

    A test that carries no `client_case` marker, or that was skipped, writes
    none, so its case stays `not_reached`. The last check of each entry is
    whether every phase passed.
    """
    del nextitem
    result = yield
    log = item.config.stash.get(_CASE_LOG, None)
    cases = item.config.stash.get(_CASES, None)
    markers = [marker.kwargs for marker in item.iter_markers(CASE_MARKER)]
    reports = item.stash.get(_REPORTS, None) or []
    if log is None or cases is None or not markers or not reports:
        return result
    if any(report.skipped for report in reports):
        return result
    lines = case_log_lines(
        markers,
        cases,
        item.stash.get(_RECORD, None),
        test_passed=all(report.passed for report in reports),
    )
    with log.open("a", encoding="utf-8") as file:
        file.write(lines)
    return result


@pytest.fixture
def client_case(request: pytest.FixtureRequest) -> ClientCaseRecord:
    """The record of the Client conformance case the test carries.

    Raises:
        ClientCaseUntagged: The test carries no `client_case` marker.
    """
    # pytest leaves `FixtureRequest.node` unannotated.
    item = cast("pytest.Item", request.node)  # pyright: ignore[reportUnknownMemberType]
    if item.get_closest_marker(CASE_MARKER) is None:
        message = (
            f"the test {item.nodeid} records a Client conformance case but carries "
            f'no `{CASE_MARKER}(id="<case>", mode="attached")` marker'
        )
        raise ClientCaseUntagged(message)
    return item.stash.setdefault(_RECORD, ClientCaseRecord())


@pytest.fixture
def proving_ground_seed() -> int | None:
    """The seed of the test's Proving ground: none unless the test overrides it."""
    return None


@pytest.fixture
def with_daemon() -> bool:
    """Whether the test's setup starts a Daemon: not unless the test overrides it."""
    return False


@pytest.fixture
def with_browser() -> bool:
    """Whether the test's engine opens a page: not unless the test overrides it."""
    return False


@pytest.fixture
def test_env(
    request: pytest.FixtureRequest,
    proving_ground_seed: int | None,
    with_daemon: bool,
    with_browser: bool,
    product: Product,
) -> Iterator[TestEnv]:
    """The test's setup from `goodfetch-test-env`, torn down after the test.

    It starts a Proving ground when `proving_ground_seed` names a seed, and
    the Daemon of `product`'s executable in the test's home when
    `with_daemon` is `True`, and lays out a Browser when `with_browser` is.
    The teardown stops the Daemon and fails the
    test when a process it started is still alive; a failed test's folder
    stays for its evidence, as a Rust test's does.
    """
    # pytest leaves `FixtureRequest.node` unannotated.
    item = cast("pytest.Item", request.node)  # pyright: ignore[reportUnknownMemberType]
    children = item.config.stash.get(_CHILDREN, None)
    env = TestEnv.open(
        item.nodeid,
        os.environ,
        proving_ground_seed=proving_ground_seed,
        daemon=product if with_daemon else None,
        browser=with_browser,
        child_variables=None if children is None else children.variables(),
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
"""Where `--collect-only` writes each test's `journey`, `covers` and `client_case`
markers."""
CASE_LOG_OPTION = "--case-log"
"""The case log of Client conformance cases a run writes."""
CASE_LIST_OPTION = "--case-list"
"""The case list the `client_case` markers name cases of."""
MUTATION_CHILDREN_OPTION = "--mutation-children"
"""Makes the Python programs tests start run mutmut's mutated copy (ADR 0072)."""


def pytest_addoption(parser: pytest.Parser) -> None:
    """Adds the `goodfetch` group's options, such as `--mutation-children`."""
    group = parser.getgroup("goodfetch")
    group.addoption(
        LISTING_OPTION,
        metavar="PATH",
        help="write each collected test's journey, covers and client_case values "
        "to PATH as JSON",
    )
    group.addoption(
        CASE_LOG_OPTION,
        metavar="PATH",
        help="write the case log of Client conformance cases to PATH, which each "
        "run starts empty; needs --case-list",
    )
    group.addoption(
        CASE_LIST_OPTION,
        metavar="PATH",
        help="the Client conformance case list each client_case marker names a "
        "case of, such as conformance/clients/cases.toml",
    )
    group.addoption(
        MUTATION_CHILDREN_OPTION,
        action="store_true",
        help="in a mutmut run, make each Python program a test starts run the "
        "mutated copy and its mutant, and count what it reaches for that test",
    )


SYNC_API_THREADS = 4
"""How many threads run each `sync_api` test at once."""


def pytest_configure(config: pytest.Config) -> None:
    """Checks the run's claims, registers the markers and the `anyio_backend` fixture.

    Raises:
        pytest.UsageError: trio does not import, `PYTHON_GIL=0` is set while
            the GIL is on, or `--case-log` names no `--case-list` or the case
            list is invalid.
    """
    _require_trio()
    _require_gil_off_when_asked(os.environ)
    _open_case_log(config)
    if config.getoption(MUTATION_CHILDREN_OPTION):
        _reach_mutation_children(config)
    # Registered once every plugin has loaded, so it overrides anyio's own
    # fixture, which drops trio when trio does not import.
    config.pluginmanager.register(_AnyioBackend(), "goodfetch_anyio_backend")
    config.addinivalue_line(
        "markers",
        "journey(id, surface): the Journey variant the test runs, as "
        'journey(id="J-002", surface="sdk-python"), which -m selects (ADR 0063)',
    )
    config.addinivalue_line(
        "markers",
        "covers(*features): the Features the test proves, as story:3-18 (ADR 0072)",
    )
    config.addinivalue_line(
        "markers",
        f"{CASE_MARKER}(id, mode): the Client conformance case the test runs and its "
        f'mode, as {CASE_MARKER}(id="hello-refused", mode="attached") (ADR 0069)',
    )
    config.addinivalue_line(
        "markers",
        f"sync_api: a unit test of the generated sync API, run in {SYNC_API_THREADS} "
        "threads at once through pytest-run-parallel (ADR 0069)",
    )


def _open_case_log(config: pytest.Config) -> None:
    """Reads the case list, and starts the case log empty on a run that runs tests.

    A relative path is the workspace's, the run's root folder. An xdist
    worker appends to the log its controller started.
    """
    log = cast("str | None", config.getoption(CASE_LOG_OPTION))
    case_list = cast("str | None", config.getoption(CASE_LIST_OPTION))
    if log is not None and case_list is None:
        message = (
            f"{CASE_LOG_OPTION} needs {CASE_LIST_OPTION}, which gives each case's "
            "capability (ADR 0069)"
        )
        raise pytest.UsageError(message)
    try:
        cases = (
            None
            if case_list is None
            else read_client_cases(config.rootpath / case_list)
        )
    except CaseListInvalid as error:
        raise pytest.UsageError(str(error)) from error
    config.stash[_CASES] = cases
    path = None if log is None else config.rootpath / log
    config.stash[_CASE_LOG] = path
    collect_only = cast("bool", config.getoption("collectonly"))
    if path is None or collect_only or hasattr(config, "workerinput"):
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("", "utf-8")


def _reach_mutation_children(config: pytest.Config) -> None:
    """Starts the run's hits folder, and names it to the programs tests start.

    A test's cleared environment map gets the variables through `test_env`;
    a program started with the runner's own environment, through it.
    """
    children = MutationChildren.start()
    config.stash[_CHILDREN] = children
    variables = children.variables()
    # The active mutant is mutmut's to set in the runner's environment.
    del variables["MUTANT_UNDER_TEST"]
    config.stash[_ENVIRON] = {name: os.environ.get(name) for name in variables}
    os.environ.update(variables)


def pytest_unconfigure(config: pytest.Config) -> None:
    """Restores the runner's environment, and removes the hits folder."""
    children = config.stash.get(_CHILDREN, None)
    if children is None:
        return
    for name, value in config.stash[_ENVIRON].items():
        if value is None:
            os.environ.pop(name, None)
        else:
            os.environ[name] = value
    children.stop()


@pytest.hookimpl(tryfirst=True)
def pytest_runtest_teardown(item: pytest.Item) -> None:
    """Counts what the test's programs reached as reached by the test.

    It runs before mutmut's own teardown hook, which gives the test every
    function counted since the last test.
    """
    children = item.config.stash.get(_CHILDREN, None)
    if children is not None:
        children.record_hits()


def _require_trio() -> None:
    try:
        importlib.import_module("trio")
    except ImportError as error:
        message = (
            f"trio does not import ({error}), so async tests would run on asyncio "
            "alone: install the workspace's dev dependencies (ADR 0069)"
        )
        raise pytest.UsageError(message) from error


def _require_gil_off_when_asked(environ: Mapping[str, str]) -> None:
    if environ.get("PYTHON_GIL") != "0":
        return
    # Python 3.12 and earlier ignore `PYTHON_GIL` and have no
    # `sys._is_gil_enabled`: their GIL is always on.
    gil_enabled: object = getattr(sys, "_is_gil_enabled", None)
    if not callable(gil_enabled) or gil_enabled():
        message = (
            "PYTHON_GIL=0 asks for a free-threaded run, but the GIL is on: run a "
            "free-threaded build without `-X gil=1`, and import no extension "
            "module that needs the GIL (ADR 0069)"
        )
        raise pytest.UsageError(message)


class _AnyioBackend:
    """Holds the `anyio_backend` fixture, which overrides anyio's."""

    @pytest.fixture(scope="module", params=ASYNC_BACKENDS)
    def anyio_backend(self, request: pytest.FixtureRequest) -> str:
        """The backend an async test runs on: asyncio, then trio."""
        return cast("str", request.param)


def pytest_collection_finish(session: pytest.Session) -> None:
    """Writes the marker listing that the Feature scanner and the Journey check read.

    It lists every collected test by node ID, with the keyword arguments of
    each of its `journey` and `client_case` markers, the arguments of its
    `covers` markers, its own nearest first, and whether a `skip` or `skipif`
    marker may keep it from running. An xdist worker never writes it;
    `--collect-only` runs no worker.
    """
    path = cast("str | None", session.config.getoption(LISTING_OPTION))
    if path is None or hasattr(session.config, "workerinput"):
        return
    tests = [
        {
            "id": item.nodeid,
            "journey": [dict(marker.kwargs) for marker in item.iter_markers("journey")],
            "covers": _marker_values(item, "covers"),
            "client_case": [
                dict(marker.kwargs) for marker in item.iter_markers(CASE_MARKER)
            ],
            "skipped": any(
                item.get_closest_marker(name) is not None for name in ("skip", "skipif")
            ),
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
    """Fails each test whose fixture closure patches or whose case is wrong.

    It also threads each `sync_api` test.

    The closure holds every fixture the test requests, directly or through
    another fixture, so a third-party fixture built on `monkeypatch` is
    refused too (ADR 0062). A `client_case` marker names a case of the case
    list, when the run reads one, and the mode `attached` or `daemon`, by
    keyword (ADR 0069). A `sync_api` test gets pytest-run-parallel's
    `force_parallel_threads` marker.
    """
    for item in items:
        if item.get_closest_marker("sync_api") is not None:
            item.add_marker(pytest.mark.force_parallel_threads(SYNC_API_THREADS))
    cases = session.config.stash.get(_CASES, None)
    refused: list[pytest.Item] = []
    for item in items:
        refusal = _patching_refusal(item) or _case_refusal(item, cases)
        if refusal is not None:
            refused.append(item)
            report = pytest.CollectReport(item.nodeid, "failed", refusal, [])
            session.ihook.pytest_collectreport(report=report)
    items[:] = [item for item in items if item not in refused]


def _patching_refusal(item: pytest.Item) -> str | None:
    patching = sorted(PATCHING_FIXTURES.intersection(_fixture_closure(item)))
    if not patching:
        return None
    return (
        f"{item.nodeid} reaches {', '.join(patching)}, directly or through "
        "another fixture: pass the value through an option instead (ADR 0062)"
    )


def _case_refusal(item: pytest.Item, cases: Mapping[str, str] | None) -> str | None:
    for marker in item.iter_markers(CASE_MARKER):
        id_ = marker.kwargs.get("id")
        if (
            marker.args
            or not set(marker.kwargs) <= {"id", "mode"}
            or not isinstance(id_, str)
        ):
            return (
                f"{item.nodeid} carries a `{CASE_MARKER}` marker that is not "
                f'`{CASE_MARKER}(id="<case>", mode="attached")` (ADR 0069)'
            )
        if marker.kwargs.get("mode") not in CLIENT_CASE_MODES:
            return (
                f"{item.nodeid} runs the case {id_} in no mode: give its "
                f'`{CASE_MARKER}` marker `mode="attached"` or `mode="daemon"` '
                "(ADR 0069)"
            )
        if cases is not None and id_ not in cases:
            return (
                f"{item.nodeid} carries the case {id_}, which the case list does "
                "not declare (ADR 0069)"
            )
    return None


def _fixture_closure(item: pytest.Item) -> list[str]:
    """Every fixture `item` reaches; an item that is no function reaches none."""
    names: object = getattr(item, "fixturenames", [])
    return cast("list[str]", names)
