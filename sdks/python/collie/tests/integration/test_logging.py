"""The host's logging (ADR 0078) on the Scripted engine as the Attached engine.

The level of the engine's logs a Connection receives follows the effective
level of the `goodfetch.engine` logger, sent in `hello` and again when a Run
starts and it changed; each `$/log` event reaches a `goodfetch.engine.<area>`
logger at its level; and a program that configures no logging sees nothing
on stdout or stderr. The Scripted engine fails on any message its script
does not expect, `hello` included, and writes that failure on stderr, which
reaches the `goodfetch.client` logger. Three ways: async on asyncio, async
on Trio and through the sync `Collie`. TypeScript's `logging.test.ts` is its
twin.
"""

import json
import logging
import subprocess
import sys
from collections.abc import Mapping
from pathlib import Path

import pytest
from goodfetch_collie import AsyncCollie, Collie
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

PROGRAMS = Path(__file__).parents[1] / "programs"
"""The folder of the programs both the unit and the integration tests run."""

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long a program the tests start may take, its engine's start included."""

CLIENT_LOGGER = "goodfetch.client"
"""The logger the engine's stderr lines and a refused log level go to."""

ENGINE_LOGGER = "goodfetch.engine"
"""The logger whose effective level the engine's logs follow."""


async def open_collie(way: Way, env: Mapping[str, str]) -> AsyncCollie:
    """A Connection this way opened on the Attached engine `env` names."""
    return await way.open(
        lambda: AsyncCollie.connect(env=env), lambda: Collie.connect(env=env)
    )


def engine_stderr(caplog: pytest.LogCaptureFixture) -> list[str]:
    """Each line the engine wrote on stderr, which a failed play writes."""
    return [
        record.getMessage()
        for record in caplog.records
        if record.name == CLIENT_LOGGER
        and getattr(record, "source", None) == "engine stderr"
    ]


def engine_events(caplog: pytest.LogCaptureFixture) -> list[logging.LogRecord]:
    """The records the engine's `$/log` events made."""
    return [
        record
        for record in caplog.records
        if getattr(record, "source", None) == "engine"
    ]


async def start_and_finish(collie: AsyncCollie, run_id: str) -> None:
    """Starts the Run `run_id` and finishes it."""
    run = await collie.start(run_id=run_id)
    await run.finish()


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
    caplog.set_level(logging.DEBUG, logger=ENGINE_LOGGER)
    collie = await open_collie(way, env)

    await start_and_finish(collie, "first")
    await start_and_finish(collie, "second")
    # The effective level: `goodfetch.engine` sets none, so `goodfetch`'s holds.
    logging.getLogger(ENGINE_LOGGER).setLevel(logging.NOTSET)
    caplog.set_level(logging.WARNING, logger="goodfetch")
    await start_and_finish(collie, "third")
    await collie.close()

    with client_case.check(
        "the engine read hello at debug, no change for the second start and one "
        "change to warn before the third, so it wrote no failure"
    ):
        assert engine_stderr(caplog) == []


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
    caplog.set_level(logging.DEBUG, logger=ENGINE_LOGGER)
    collie = await open_collie(way, env)

    caplog.set_level(logging.WARNING, logger=ENGINE_LOGGER)
    await start_and_finish(collie, "first")
    await start_and_finish(collie, "second")
    await collie.close()

    refused = [
        (record.levelno, getattr(record, "log_level", None))
        for record in caplog.records
        if record.name == CLIENT_LOGGER and hasattr(record, "log_level")
    ]
    assert (refused, engine_stderr(caplog)) == ([(logging.WARNING, "warn")], [])


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
    caplog.set_level(1, logger=ENGINE_LOGGER)
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
        "each event reached its area's logger at its level, trace below DEBUG"
    ):
        assert [(event["logger"], event["level"]) for event in events] == [
            ("goodfetch.engine.rpc", logging.ERROR),
            ("goodfetch.engine.page", logging.WARNING),
            ("goodfetch.engine.run", logging.INFO),
            ("goodfetch.engine.rpc", logging.DEBUG),
            ("goodfetch.engine.page", 5),
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
        assert engine_stderr(caplog) == []


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

    record = test_env.home / "scripted-engine" / "record.jsonl"
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
    assert "goodfetch.engine.rpc" in loggers
    assert loggers == {name: [logging.NOTSET, [], True] for name in loggers}
