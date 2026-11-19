"""Collie's Connection on an Attached engine (ADR 0069).

The engine is the executable of the same commit, which `env` points at
with `GOODFETCH_COLLIE_ENGINE_PATH` and the test's own Goodfetch home, or
the Scripted engine started as that executable. The tracer program is the
one the unit tests run on the Scripted engine over the in-memory transport,
three ways: async on asyncio, async on Trio and through the sync `Collie`.
The host's logging (ADR 0078) is played here too: the level of the engine's
logs follows the effective level of `goodfetch.engine`, sent in `hello` and
again when a Run starts and it changed, each `$/log` event reaches the
`goodfetch.engine` logger its target names, and a program that configures
no logging sees nothing on stdout or stderr. TypeScript's `attached.test.ts`
and `logging.test.ts` are its twins.
"""

import json
import logging
import os
import re
import shutil
import subprocess
import sys
from collections.abc import Mapping
from datetime import timedelta
from pathlib import Path

import anyio
import pytest
from collie_calls import CLIENT_LOGGER, engine_warnings, failure, open_collie
from collie_tracer import Traced, start_and_finish
from goodfetch_collie import (
    AsyncCollie,
    ClosedError,
    EngineExitedError,
    EngineInfo,
    EngineOutputInvalidError,
    Limits,
    Product,
    SettingInvalidError,
    StartedAs,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    scripted_engine_record,
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

ENGINE_EVENTS_LOGGER = "goodfetch.engine"
"""The logger whose effective level the engine's logs follow."""


def engine_events(caplog: pytest.LogCaptureFixture) -> list[logging.LogRecord]:
    """The records the engine's `$/log` events made."""
    return [
        record
        for record in caplog.records
        if getattr(record, "source", None) == "engine"
    ]


async def start_and_finish_run(collie: AsyncCollie, run_id: str) -> None:
    """Starts the Run `run_id` on `collie` and finishes it, leaving it open."""
    run = await collie.start(run_id=run_id)
    await run.finish()


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
async def test_sends_a_timeout_in_its_three_states(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Left out it is not sent, `None` sends null, a `timedelta` its milliseconds.

    The script expects each Run's `run.finish` with no `timeout`, with null
    and with 2 ms, 1.5 ms rounded up (ADR 0080).
    """
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/call-timeout-three-states.json"
    )
    collie = await open_collie(way, env)

    default = await (await collie.start()).finish()
    none = await (await collie.start()).finish(timeout=None)
    given = await (await collie.start()).finish(timeout=timedelta(microseconds=1500))
    await collie.close()

    with client_case.check("each state of the timeout reached the engine as sent"):
        assert [report.lifecycle for report in (default, none, given)] == [
            "finished",
            "finished",
            "finished",
        ]
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-40", "story:11-42")
@pytest.mark.client_case(id="call-timeout-three-states", mode="attached")
@pytest.mark.parametrize(
    ("given", "value"),
    [
        pytest.param(timedelta(0), 0, id="zero"),
        pytest.param(timedelta(seconds=-1), -1000, id="negative"),
        pytest.param(timedelta(microseconds=-1), -0.001, id="negative-below-a-ms"),
    ],
)
@three_ways
async def test_refuses_a_timeout_of_zero_or_less_before_sending_it(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
    given: timedelta,
    value: float,
) -> None:
    """The refused `finish` sends nothing.

    The script's one `run.finish` has no `timeout`.
    """
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/started-and-finished.json"
    )
    collie = await open_collie(way, env)

    run = await collie.start()
    refused = await failure(run.finish(timeout=given))
    await run.finish()
    await collie.close()

    with client_case.check(
        "a timeout of zero or less is refused before it is sent, naming its setting"
        " and the null form"
    ):
        assert isinstance(refused, SettingInvalidError)
        assert refused.details.get("limit") == {
            "key": "timeout.run",
            "value": value,
            "layer": "run",
            "origin": "argument",
        }
        assert "`timeout=None`" in (refused.fix or "")
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
    """Two Connections, each with its own engine, home and settings, run at once.

    The second Connection alone sets a Limit: its own engine expects the
    `usage.set`, and the first's would fail on it.
    """
    traced: list[Traced] = []

    async def trace(home: str, script: str, limits: Limits | None) -> None:
        env = scripted_engine_env(
            test_env, product, f"sdk-python/run/{script}", test_env.folder / home
        )
        collie = await open_collie(way, env, limits)
        traced.append(await start_and_finish(collie))

    async with anyio.create_task_group() as group:
        group.start_soon(trace, "first", "started-and-finished.json", None)
        group.start_soon(
            trace,
            "second",
            "started-and-finished-under-a-limit.json",
            Limits(model_calls=5),
        )

    with client_case.check("each Connection's Run finished"):
        assert [each.report.lifecycle for each in traced] == ["finished", "finished"]
    with client_case.check(
        "each engine got its own Connection's settings, the Limit the second's"
        " alone, and played its script to its end"
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


@pytest.mark.covers("story:11-26")
@pytest.mark.client_case(id="log-level-follows-the-logger", mode="attached")
@three_ways
async def test_sends_the_level_in_hello_and_again_before_a_start_once_it_changed(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """`DEBUG` on `goodfetch.engine`, then `WARNING` inherited from `goodfetch`."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/log-level-follows-the-logger.json"
    )
    caplog.set_level(logging.DEBUG, logger=ENGINE_EVENTS_LOGGER)
    collie = await open_collie(way, env)

    await start_and_finish_run(collie, "first")
    await start_and_finish_run(collie, "second")
    # The effective level: `goodfetch.engine` sets none, so `goodfetch`'s holds.
    logging.getLogger(ENGINE_EVENTS_LOGGER).setLevel(logging.NOTSET)
    caplog.set_level(logging.WARNING, logger="goodfetch")
    await start_and_finish_run(collie, "third")
    await collie.close()

    with client_case.check(
        "the engine read hello at debug, no change for the second start and one "
        "change to warn before the third, so it wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-26")
@three_ways
async def test_goes_on_after_a_refused_level_and_tells_the_engine_again(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The refusal warns on `goodfetch.client`; the next start tells it again."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/log-level-refused.json"
    )
    caplog.set_level(logging.DEBUG, logger=ENGINE_EVENTS_LOGGER)
    collie = await open_collie(way, env)

    caplog.set_level(logging.WARNING, logger=ENGINE_EVENTS_LOGGER)
    await start_and_finish_run(collie, "first")
    await start_and_finish_run(collie, "second")
    await collie.close()

    # The refusal alone: the engine wrote no failure on stderr.
    assert [
        (record.levelno, getattr(record, "log_level", None))
        for record in caplog.records
        if record.name == CLIENT_LOGGER
    ] == [(logging.WARNING, "warn")]


@pytest.mark.covers("story:11-25")
@pytest.mark.client_case(id="engine-logs-reach-the-logger", mode="attached")
@three_ways
async def test_hands_each_of_the_engines_log_events_to_its_area_logger_at_its_level(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Error, warning, info, debug and trace, each with its target, fields and drops."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/engine-logs-reach-the-logger.json"
    )
    caplog.set_level(1, logger=ENGINE_EVENTS_LOGGER)
    collie = await open_collie(way, env)
    await collie.close()

    events = [
        {
            "logger": record.name,
            "level": record.levelno,
            "target": getattr(record, "target", None),
            "fields": getattr(record, "fields", None),
            "dropped": getattr(record, "dropped", None),
        }
        for record in engine_events(caplog)
    ]
    client_case.message(events)
    with client_case.check(
        "each event reached its target's logger at its level, trace below DEBUG"
    ):
        assert [(event["logger"], event["level"]) for event in events] == [
            ("goodfetch.engine.rpc.dispatch", logging.ERROR),
            ("goodfetch.engine.page", logging.WARNING),
            ("goodfetch.engine.run", logging.INFO),
            ("goodfetch.engine.rpc.dispatch", logging.DEBUG),
            ("goodfetch.engine.page.cdp", 5),
        ]
    with client_case.check(
        "each event kept its target, its fields and its dropped count"
    ):
        assert [
            (event["target"], event["fields"], event["dropped"]) for event in events
        ] == [
            ("goodfetch::rpc::dispatch", {"operation": "run.start"}, 0),
            ("goodfetch::page", {}, 3),
            ("goodfetch::run", {}, 0),
            ("goodfetch::rpc::dispatch", {"method": "run.start", "id": 7}, 0),
            ("goodfetch::page::cdp", {}, 0),
        ]
    with client_case.check("the engine read hello at trace, so it wrote no failure"):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-25")
@pytest.mark.parametrize("way", ["asyncio", "trio", "sync"])
def test_writes_nothing_to_stdout_or_stderr_in_a_program_that_configures_no_logging(
    way: str, test_env: TestEnv, product: ToolProduct
) -> None:
    """The engine's error and warning reach `goodfetch`'s `NullHandler` alone."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/engine-logs-without-logging.json"
    )
    out = test_env.folder / "loggers.json"

    program = subprocess.run(
        [sys.executable, str(PROGRAMS / "collie_unlogged.py"), str(out), way],
        capture_output=True,
        cwd=test_env.folder,
        env=dict(env),
        timeout=PROGRAM_DEADLINE_SECONDS,
        check=False,
    )

    record = scripted_engine_record(test_env.home)
    sent = [
        line["message"]["params"]["level"]
        for line in map(json.loads, record.read_text("utf-8").splitlines())
        if line.get("from") == "engine"
        and line.get("message", {}).get("method") == "$/log"
    ]
    assert {
        "exit": program.returncode,
        "stdout": program.stdout.decode(errors="replace"),
        "stderr": program.stderr.decode(errors="replace"),
        "sent": sent,
    } == {"exit": 0, "stdout": "", "stderr": "", "sent": ["error", "warn"]}
    loggers = json.loads(out.read_text("utf-8"))
    assert loggers.pop("goodfetch") == [logging.NOTSET, ["NullHandler"], True]
    assert "goodfetch.engine.rpc.dispatch" in loggers
    assert loggers == {name: [logging.NOTSET, [], True] for name in loggers}
