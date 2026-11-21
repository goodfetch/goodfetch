"""The host's logging (ADR 0078): the engine's level and areas, from `logging`.

The level of the engine's logs a Connection asks for follows the effective
level of `goodfetch.engine`; each event goes to the logger of its target's
area. The integration tests play both on the Scripted engine; these cover
every level's edge and every target's shape. TypeScript's `logger.test.ts`
is its twin.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import pytest
from goodfetch_collie_client.generated.models import LogLevel
from goodfetch_collie_client.host_logging import (
    ENGINE_LOGGER,
    engine_logger_name,
    log_engine_event,
    log_level,
)

if TYPE_CHECKING:
    from collections.abc import Iterator


@pytest.fixture
def engine_logger() -> Iterator[logging.Logger]:
    """`goodfetch.engine`, its level and filters restored afterwards."""
    logger = logging.getLogger(ENGINE_LOGGER)
    level, filters = logger.level, list(logger.filters)
    yield logger
    logger.setLevel(level)
    logger.filters[:] = filters


@pytest.mark.covers("story:11-26")
def test_maps_each_effective_level_of_the_engine_logger_to_the_engines_level(
    engine_logger: logging.Logger,
) -> None:
    """Each `logging` level's edge, from above `CRITICAL` to below `DEBUG`."""
    levels = {}
    for level in (51, 50, 41, 40, 31, 30, 21, 20, 11, 10, 9, 5, 1):
        engine_logger.setLevel(level)
        levels[level] = log_level()

    assert levels == {
        51: LogLevel.off,
        50: LogLevel.error,
        41: LogLevel.error,
        40: LogLevel.error,
        31: LogLevel.error,
        30: LogLevel.warn,
        21: LogLevel.warn,
        20: LogLevel.info,
        11: LogLevel.info,
        10: LogLevel.debug,
        9: LogLevel.trace,
        5: LogLevel.trace,
        1: LogLevel.trace,
    }


@pytest.mark.covers("story:11-25")
def test_names_the_logger_of_an_event_after_its_targets_path() -> None:
    """Without a leading `goodfetch`, or `goodfetch.engine` without a valid path."""
    targets = [
        "goodfetch::rpc::dispatch",
        "goodfetch::page",
        "goodfetch_frontier::robots_txt",
        "goodfetch_engine::robots_txt",
        "goodfetch",
        "goodfetch::",
        "goodfetch::rpc::",
        "goodfetch::not a path",
        "site.test:443",
        "",
    ]

    assert [engine_logger_name(target) for target in targets] == [
        "goodfetch.engine.rpc.dispatch",
        "goodfetch.engine.page",
        "goodfetch.engine.goodfetch_frontier.robots_txt",
        "goodfetch.engine.goodfetch_engine.robots_txt",
        "goodfetch.engine",
        "goodfetch.engine",
        "goodfetch.engine",
        "goodfetch.engine",
        "goodfetch.engine",
        "goodfetch.engine",
    ]


@pytest.mark.covers("story:11-25")
def test_drops_an_event_it_cannot_log_and_one_a_hosts_filter_raises_on(
    engine_logger: logging.Logger, caplog: pytest.LogCaptureFixture
) -> None:
    """`off`, an unknown level, a missing target or message, and a raising filter."""
    caplog.set_level(1, logger=ENGINE_LOGGER)
    event = {"level": "warn", "target": "goodfetch", "message": "Slow."}
    unloggable: list[object] = [
        None,
        [],
        {**event, "level": "off"},
        {**event, "level": "fatal"},
        {"level": "warn", "message": "Slow."},
        {"level": "warn", "target": "goodfetch"},
    ]
    refused: list[str] = []

    def refuse(record: logging.LogRecord) -> bool:
        refused.append(record.getMessage())
        raise RuntimeError(record.getMessage())

    log_engine_event(event)
    for params in unloggable:
        log_engine_event(params)
    # A target without a path logs on `goodfetch.engine`, whose filter runs.
    engine_logger.addFilter(refuse)
    log_engine_event({**event, "message": "Refused."})

    assert (
        [
            (record.name, record.levelno, record.getMessage())
            for record in caplog.records
        ],
        refused,
    ) == ([(ENGINE_LOGGER, logging.WARNING, "Slow.")], ["Refused."])
