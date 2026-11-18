"""Collie's Connection on an Attached engine (ADR 0069).

The engine is the executable of the same commit, which `env` points at
with `GOODFETCH_COLLIE_ENGINE_PATH` and the test's own Goodfetch home, or
the Scripted engine started as that executable. The tracer program is the
one the unit tests run on the Scripted engine over the in-memory transport,
three ways: async on asyncio, async on Trio and through the sync `Collie`.
TypeScript's `attached.test.ts` is its twin.
"""

import json
import os
import re
import shutil
import subprocess
import sys
from collections.abc import Awaitable, Mapping
from datetime import timedelta
from pathlib import Path

import anyio
import pytest
from collie_tracer import Traced, start_and_finish
from goodfetch_collie import (
    AsyncCollie,
    ClosedError,
    Collie,
    EngineExitedError,
    EngineInfo,
    EngineOutputInvalidError,
    GoodfetchError,
    Product,
    SettingInvalidError,
    StartedAs,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct
from inline_snapshot import snapshot

RUN_ID = re.compile(r"^[0-9a-z]{26}$")

PROGRAMS = Path(__file__).parents[1] / "programs"
"""The folder of the programs both the unit and the integration tests run."""

CLIENT_SOURCE = Path(__file__).parents[3] / "collie-client"
"""The Collie client package's source, whose wheel the tests build."""

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long a program the tests start may take, its engine's start included."""

BUILD_DEADLINE_SECONDS = 120.0
"""How long one wheel's build may take."""

ENGINE_LOGGER = "goodfetch.client"
"""The logger the engine's stderr lines go to."""


async def open_collie(way: Way, env: Mapping[str, str]) -> AsyncCollie:
    """A Connection this way opened on the Attached engine `env` names."""
    return await way.open(
        lambda: AsyncCollie.connect(env=env), lambda: Collie.connect(env=env)
    )


def engine_warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
    """Each line the engines wrote on stderr, which a failed play writes."""
    return [
        record.getMessage() for record in caplog.records if record.name == ENGINE_LOGGER
    ]


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


@pytest.mark.covers("story:11-54")
@three_ways
async def test_starts_a_run_finishes_it_and_answers_its_run_report(
    way: Way, env: Mapping[str, str], test_env: TestEnv
) -> None:
    """The tracer program runs the same on the executable of the same commit."""
    collie = await open_collie(way, env)
    engine = collie.engine

    traced = await start_and_finish(collie)

    assert engine == EngineInfo(
        kind="attached",
        product=Product.collie,
        version="0.0.0",
        protocol_version="0.1",
        engine_path=env["GOODFETCH_COLLIE_ENGINE_PATH"],
    )
    assert traced.started_as == StartedAs.new
    assert RUN_ID.match(traced.run_id), traced.run_id
    assert (
        traced.report.run_id.root,
        traced.report.lifecycle,
        traced.report.verdict,
        traced.report.completeness,
    ) == (traced.run_id, "finished", "verified", "complete")
    assert (test_env.home / "0.0" / "runs" / traced.run_id / "run.sqlite").exists()


@pytest.mark.client_case(id="nothing-at-import-or-connect", mode="attached")
@pytest.mark.parametrize("way", ["asyncio", "trio", "sync"])
def test_imports_connects_and_closes_as_a_guest(
    way: str, env: Mapping[str, str], test_env: TestEnv, client_case: ClientCaseRecord
) -> None:
    """Nothing is read, patched or left running, by import, connect or close."""
    out = test_env.folder / "guest.json"

    guest = subprocess.run(
        [sys.executable, str(PROGRAMS / "collie_guest.py"), str(out), way],
        capture_output=True,
        cwd=test_env.folder,
        env=dict(env),
        timeout=PROGRAM_DEADLINE_SECONDS,
        check=False,
    )

    with client_case.check("the program ended at once with exit code 0"):
        assert (guest.returncode, guest.stderr.decode(errors="replace")) == (0, "")
    with client_case.check("the SDK wrote nothing to stdout"):
        assert guest.stdout == b""
    seen = json.loads(out.read_text("utf-8"))
    client_case.message(seen)
    with client_case.check("importing read no variable"):
        assert (seen["read_at_import"], seen["lifecycle"]) == ([], "finished")
    with client_case.check("no handler or hook was added at import or connect"):
        assert seen["after_import"] == seen["before_import"]
        assert seen["after_close"] == seen["before_connect"]


@pytest.mark.covers("story:11-54")
def test_a_sync_collie_left_unclosed_keeps_no_thread_that_holds_the_process(
    env: Mapping[str, str], test_env: TestEnv
) -> None:
    """A program that forgets `close()` still exits, its engine with it."""
    unclosed = subprocess.run(
        [sys.executable, str(PROGRAMS / "collie_unclosed.py")],
        capture_output=True,
        cwd=test_env.folder,
        env=dict(env),
        timeout=PROGRAM_DEADLINE_SECONDS,
        check=False,
    )

    assert (unclosed.returncode, json.loads(unclosed.stdout)) == (0, []), (
        unclosed.stderr.decode(errors="replace")
    )


@pytest.mark.covers("story:11-248")
def test_importing_the_sdk_loads_no_adapter_ai_provider_or_browser_driver(
    test_env: TestEnv,
) -> None:
    """The import cost (ADR 0071): each package outside the standard library loaded.

    No adapter, AI provider or Playwright is among them. `sysconfig`'s data
    module, named after the platform, is the standard library's own.
    """
    loaded = subprocess.run(
        [
            sys.executable,
            "-c",
            "import json, sys\n"
            "before = set(sys.modules)\n"
            "import goodfetch_collie\n"
            "top = {name.partition('.')[0] for name in set(sys.modules) - before}\n"
            "print(json.dumps(sorted(\n"
            "    name for name in top - set(sys.stdlib_module_names)\n"
            "    if not name.startswith('_sysconfigdata_')\n"
            ")))\n",
        ],
        capture_output=True,
        cwd=test_env.folder,
        env=dict(test_env.env),
        timeout=PROGRAM_DEADLINE_SECONDS,
        check=True,
    )

    assert json.loads(loaded.stdout) == snapshot(
        [
            "annotated_types",
            "anyio",
            "goodfetch_collie",
            "goodfetch_collie_client",
            "pydantic",
            "pydantic_core",
            "sniffio",
            "typing_extensions",
            "typing_inspection",
        ]
    )


def test_the_client_wheel_is_pure_python(test_env: TestEnv) -> None:
    """It holds no compiled code, so one wheel serves every system."""
    source = test_env.folder / "collie-client"
    shutil.copytree(
        CLIENT_SOURCE,
        source,
        ignore=shutil.ignore_patterns("__pycache__", "tests"),
    )
    out = test_env.folder / "wheels"
    uv = shutil.which("uv", path=os.environ.get("PATH"))
    assert uv is not None, "uv is not on PATH"

    built = subprocess.run(
        [
            uv,
            "build",
            "--wheel",
            "--offline",
            "--no-config",
            "--python",
            sys.executable,
            "--out-dir",
            str(out),
            str(source),
        ],
        capture_output=True,
        env={
            **test_env.env,
            "PATH": os.environ.get("PATH", ""),
            "UV_CACHE_DIR": str(test_env.folder / "build cache"),
            "UV_PYTHON_DOWNLOADS": "never",
        },
        timeout=BUILD_DEADLINE_SECONDS,
        check=False,
    )

    assert built.returncode == 0, built.stderr.decode(errors="replace")
    assert [wheel.name for wheel in out.glob("*.whl")] == [
        "goodfetch_collie_client-0.0.0-py3-none-any.whl"
    ]


@pytest.mark.client_case(id="collie-accepted-by-foxhound", mode="attached")
@three_ways
async def test_reports_the_engine_it_reached_of_either_product(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A Foxhound engine serves a Collie Connection (ADR 0095)."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/accepted-by-foxhound.json"
    )
    collie = await open_collie(way, env)

    engine = collie.engine
    await collie.close()

    with client_case.check("the engine names its Product and Product version"):
        assert engine == EngineInfo(
            kind="attached",
            product=Product.foxhound,
            version="0.0.0",
            protocol_version="0.1",
            engine_path=env["GOODFETCH_COLLIE_ENGINE_PATH"],
        )
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-178")
@pytest.mark.client_case(id="catalog-mapping", mode="attached")
@three_ways
async def test_reaches_the_engine_through_the_members_generated_from_the_catalog(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """Each member a Run takes from the catalog reaches its operation."""
    collie = await open_collie(way, env)
    run = await collie.start()

    progress_set = await run.set_progress_message("three pages left")
    progress_read = await run.progress()
    finished = await run.finish()
    report = await run.report()
    await collie.close()

    with client_case.check(
        "a member with a positional parameter reaches its operation"
    ):
        assert (progress_set.run_id.root, progress_set.lifecycle) == (run.id, "running")
    with client_case.check("a member with only options answers what the engine holds"):
        assert progress_read.message == progress_set.message
    with client_case.check("a member that ends the Run answers its report"):
        assert (finished.run_id.root, finished.lifecycle) == (run.id, "finished")
    with client_case.check("the engine holds what the members did"):
        assert (report.run_id.root, report.lifecycle) == (run.id, "finished")


@pytest.mark.client_case(id="closed-after-close", mode="attached")
@three_ways
async def test_closes_a_running_run_by_aborting_it_and_refuses_every_later_call(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Each close is idempotent, and every later call fails with `closed`."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/closed-while-running.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start(run_id="nightly", tags=["shop"])

    await run.close()
    await run.close()
    await collie.close()
    await collie.close()

    with client_case.check("the Run's finish fails with closed"):
        finished = await failure(run.finish())
        assert isinstance(finished, ClosedError)
        assert (finished.code, finished.group) == ("closed", "program")
    with client_case.check("a start fails with closed"):
        started = await failure(collie.start())
        assert isinstance(started, ClosedError)
        assert (started.code, started.group) == ("closed", "program")
    with client_case.check(
        "the close aborted the Run once, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-40", "story:11-42")
@pytest.mark.client_case(id="call-timeout-three-states", mode="attached")
@three_ways
async def test_sends_a_timeout_in_its_three_states_and_refuses_one_of_zero_or_less(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Left out it is not sent, `None` sends null, a `timedelta` its milliseconds.

    The script expects each Run's `run.finish` with no `timeout`, with null
    and with 2 ms (ADR 0080); a `timeout` refused before it is sent would
    fail the script.
    """
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/call-timeout-three-states.json"
    )
    collie = await open_collie(way, env)

    default = await (await collie.start()).finish()
    none = await (await collie.start()).finish(timeout=None)
    run = await collie.start()
    refused = [
        await failure(run.finish(timeout=timeout))
        for timeout in (timedelta(0), timedelta(seconds=-1), timedelta(microseconds=-1))
    ]
    given = await run.finish(timeout=timedelta(microseconds=1500))
    await collie.close()

    with client_case.check("each state of the timeout reached the engine as sent"):
        assert [report.lifecycle for report in (default, none, given)] == [
            "finished",
            "finished",
            "finished",
        ]
    with client_case.check(
        "a timeout of zero or less is refused before it is sent, naming its setting"
        " and the null form"
    ):
        assert [
            (
                type(error),
                error.details.get("limit"),
                "`timeout=None`" in (error.fix or ""),
            )
            for error in refused
        ] == [
            (
                SettingInvalidError,
                {
                    "key": "timeout.run",
                    "value": value,
                    "layer": "run",
                    "origin": "argument",
                },
                True,
            )
            for value in (0, -1000, -0.001)
        ]
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.client_case(id="two-connections-at-once", mode="attached")
@three_ways
async def test_runs_beside_another_connection_in_the_same_process(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Two Connections, each with its own engine and home, run at once."""
    traced: list[Traced] = []

    async def trace(home: str) -> None:
        env = scripted_engine_env(
            test_env,
            product,
            "sdk-python/run/started-and-finished.json",
            test_env.folder / home,
        )
        traced.append(await start_and_finish(await open_collie(way, env)))

    async with anyio.create_task_group() as group:
        group.start_soon(trace, "first")
        group.start_soon(trace, "second")

    with client_case.check("each Connection's Run finished"):
        assert [each.report.lifecycle for each in traced] == ["finished", "finished"]
    with client_case.check(
        "each script played to its end, so no engine wrote a failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.client_case(id="connect-after-close", mode="attached")
@three_ways
async def test_connects_again_after_a_close(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A closed Connection leaves nothing that stops the next one."""
    first_env = scripted_engine_env(
        test_env,
        product,
        "sdk-python/connect/accepted-by-foxhound.json",
        test_env.folder / "first",
    )
    await (await open_collie(way, first_env)).close()

    second_env = scripted_engine_env(
        test_env,
        product,
        "sdk-python/run/started-and-finished.json",
        test_env.folder / "second",
    )
    traced = await start_and_finish(await open_collie(way, second_env))

    with client_case.check("the second Connection's Run finished"):
        assert traced.report.lifecycle == "finished"
    with client_case.check(
        "both scripts played to their end, so no engine wrote a failure"
    ):
        assert engine_warnings(caplog) == []


@three_ways
async def test_fails_the_call_in_flight_with_the_engines_exit_code(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """An engine that exits mid-call fails it with `engine_exited`."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/exit-during-a-call.json"
    )
    collie = await open_collie(way, env)

    error = await failure(collie.start())
    await collie.close()

    assert isinstance(error, EngineExitedError)
    assert (error.code, error.group, error.details) == (
        "engine_exited",
        "unavailable",
        {"exit_code": 101, "signal": None, "stderr_tail": ""},
    )


@three_ways
async def test_stops_the_engine_once_it_writes_a_line_that_is_not_json_rpc(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The call in flight fails with `engine_output_invalid`, naming the line."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/attached-engine/garbage-during-a-start.json"
    )
    collie = await open_collie(way, env)

    error = await failure(collie.start())
    await collie.close()

    assert isinstance(error, EngineOutputInvalidError)
    assert (error.code, error.group, error.details) == (
        "engine_output_invalid",
        "internal",
        {"line": "INFO engine started"},
    )
    # An engine left running would fail its play on stderr once its input ends.
    assert engine_warnings(caplog) == []
