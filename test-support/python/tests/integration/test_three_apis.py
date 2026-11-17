"""Python's three APIs stay equal, and each run tests what it claims (ADR 0069)."""

import sys
import sysconfig
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import TestEnv

if TYPE_CHECKING:
    from conftest import SampleRun

USAGE_ERROR = 4
"""pytest's exit code when the session refuses to start."""

FREE_THREADED = bool(sysconfig.get_config_var("Py_GIL_DISABLED"))
"""Whether this interpreter is a free-threaded build."""

GIL_OPTION_IGNORED = sys.version_info < (3, 13)
"""Whether this interpreter ignores `PYTHON_GIL` and `-X gil`, as 3.11 and 3.12 do."""

BACKENDS = """
import pytest
import sniffio


@pytest.mark.anyio
async def test_backend() -> None:
    assert sniffio.current_async_library() in {"asyncio", "trio"}
"""

TRIO_MISSING = "raise ImportError('trio is not installed')\n"
"""A `trio` package that fails to import, shadowing the installed one."""


def test_an_anyio_test_runs_on_asyncio_and_on_trio(sample_run: "SampleRun") -> None:
    """The plugin's `anyio_backend` names both backends, rather than probing."""
    outcome = sample_run.run(BACKENDS)

    assert (outcome.status, outcome.results) == (
        0,
        {"test_backend[asyncio]": "passed", "test_backend[trio]": "passed"},
    ), outcome.output


def test_a_session_where_trio_cannot_import_fails(
    sample_run: "SampleRun", test_env: TestEnv, capfd: pytest.CaptureFixture[str]
) -> None:
    """The plugin refuses the run, where anyio's own fixture would drop trio silently.

    pytest writes the refusal to stderr, which the sample run shares with
    this test.
    """
    shadow = test_env.folder / "shadow"
    (shadow / "trio").mkdir(parents=True)
    (shadow / "trio" / "__init__.py").write_text(TRIO_MISSING, "utf-8")

    outcome = sample_run.run(BACKENDS, env={"PYTHONPATH": str(shadow)})

    assert outcome.status == USAGE_ERROR, outcome.output
    assert "trio does not import" in capfd.readouterr().err


THREE_WAYS = """
import threading

import sniffio
from goodfetch_test_support import Way, three_ways


class AsyncApi:
    async def echo(self, value: str) -> str:
        return f"{{sniffio.current_async_library()}}:{{value}}"


class SyncApi:
    def echo(self, value: str) -> str:
        on_main = threading.current_thread() is threading.main_thread()
        return f"{sync_answer}:{{value}}" if not on_main else "main thread"


@three_ways
async def test_echo(way: Way) -> None:
    api = way.api(AsyncApi(), SyncApi())

    assert await api.echo("hi") == f"{{way.value}}:hi"
"""


@pytest.mark.parametrize(
    ("sync_answer", "sync_result"),
    [
        pytest.param("sync", "passed", id="equal"),
        pytest.param("async", "failure", id="sync-api-differs"),
    ],
)
def test_a_three_ways_case_runs_as_three_named_cases(
    sample_run: "SampleRun", sync_answer: str, sync_result: str
) -> None:
    """The async ways call the async API; the sync way calls the sync API off the loop.

    A sync API whose answer differs from the async one fails its case alone.
    """
    outcome = sample_run.run(THREE_WAYS.format(sync_answer=sync_answer))

    assert outcome.results == {
        "test_echo[asyncio]": "passed",
        "test_echo[trio]": "passed",
        "test_echo[sync]": sync_result,
    }, outcome.output


PASSING = """
def test_nothing() -> None:
    pass
"""


@pytest.mark.skipif(
    not (GIL_OPTION_IGNORED or FREE_THREADED),
    reason="PYTHON_GIL=0 stops a default build of 3.13 or later before pytest starts",
)
def test_a_run_with_python_gil_0_and_the_gil_on_fails(
    sample_run: "SampleRun", capfd: pytest.CaptureFixture[str]
) -> None:
    """`-X gil=1` keeps a free-threaded build's GIL on; 3.11 and 3.12 never drop it."""
    outcome = sample_run.run(
        PASSING, env={"PYTHON_GIL": "0"}, interpreter=["-X", "gil=1"]
    )

    assert outcome.status == USAGE_ERROR, outcome.output
    assert "the GIL is on" in capfd.readouterr().err


@pytest.mark.skipif(
    not FREE_THREADED, reason="only a free-threaded build drops the GIL"
)
def test_a_run_with_python_gil_0_on_a_free_threaded_build_passes(
    sample_run: "SampleRun",
) -> None:
    """The Release suite's free-threaded cells run this one."""
    outcome = sample_run.run(PASSING, env={"PYTHON_GIL": "0"})

    assert (outcome.status, outcome.results) == (
        0,
        {"test_nothing": "passed"},
    ), outcome.output


SHARED_STATE = """
import contextlib
import threading
import time

import pytest

LOCK = {lock}
OWNER: list[int] = []


{marker}
def test_owner_stays_mine() -> None:
    with LOCK:
        OWNER[:] = [threading.get_ident()]
        time.sleep(0.05)
        assert OWNER == [threading.get_ident()]
"""


@pytest.mark.parametrize(
    ("marker", "lock", "result"),
    [
        pytest.param(
            "@pytest.mark.sync_api", "threading.Lock()", "passed", id="locked"
        ),
        pytest.param(
            "@pytest.mark.sync_api", "contextlib.nullcontext()", "failure", id="racing"
        ),
        pytest.param(
            "", "contextlib.nullcontext()", "passed", id="racing-in-one-thread"
        ),
    ],
)
def test_a_sync_api_test_runs_in_parallel_threads_and_catches_a_race(
    sample_run: "SampleRun", marker: str, lock: str, result: str
) -> None:
    """pytest-run-parallel runs a `sync_api` test in several threads at once.

    Shared state written without a lock fails there, though one thread alone
    passes; a lock makes it pass.
    """
    outcome = sample_run.run(SHARED_STATE.format(marker=marker, lock=lock))

    assert outcome.results == {"test_owner_stays_mine": result}, outcome.output
