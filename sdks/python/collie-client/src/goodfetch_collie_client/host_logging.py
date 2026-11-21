"""The host's logging (ADR 0078): the SDK's logger names, and the engine's logs.

Every logger sits under `goodfetch`, which the package gives a `NullHandler`
and nothing else; no handler, level or root logger is touched. The names
below are SDK API under semver, listed in the SDK's manifest; message text
is not. The level of the engine's logs a Connection receives follows the
effective level of `goodfetch.engine`, and each `$/log` event goes to a
`goodfetch.engine.<area>` logger at its level, the area being its target's
path. TypeScript's `logger.ts` is
its twin.
"""

from __future__ import annotations

import logging
from typing import Final

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.generated.models import LogEvent, LogLevel

CLIENT_LOGGER: Final = "goodfetch.client"
"""The SDK's runtime: the engine reached, at info.

And at warning, the engine's stderr lines and a refused log level.
"""

RUN_LOGGER: Final = "goodfetch.client.run"
"""A start that resumed its Run, at info, or found it ended, at warning."""

ENGINE_LOGGER: Final = "goodfetch.engine"
"""The engine's log events, each under `goodfetch.engine.<area>`.

Its effective level is the level of the engine's logs a Connection asks for.
"""

TRACE: Final = 5
"""The level an engine `trace` event is logged at, below `DEBUG`.

Python names no such level; 5 is the one other libraries use for it.
"""

_LEVELS: Final = {
    LogLevel.error: logging.ERROR,
    LogLevel.warn: logging.WARNING,
    LogLevel.info: logging.INFO,
    LogLevel.debug: logging.DEBUG,
    LogLevel.trace: TRACE,
}
"""The `logging` level each engine level other than `off` is logged at."""


def log_level() -> LogLevel:
    """The level of the engine's logs the `goodfetch.engine` logger records.

    From its effective level: `CRITICAL` and `ERROR` give `error`, `WARNING`
    `warn`, `INFO` `info`, `DEBUG` `debug`, and anything below `trace`; a
    level above `CRITICAL` records none of them, so it gives `off`.
    """
    level = logging.getLogger(ENGINE_LOGGER).getEffectiveLevel()
    if level > logging.CRITICAL:
        return LogLevel.off
    if level > logging.WARNING:
        return LogLevel.error
    if level > logging.INFO:
        return LogLevel.warn
    if level > logging.DEBUG:
        return LogLevel.info
    if level == logging.DEBUG:
        return LogLevel.debug
    return LogLevel.trace


def engine_logger_name(target: str) -> str:
    """The logger an event of the engine's `target` goes to.

    `goodfetch.engine` followed by the target's path, each `::` a dot and a
    leading `goodfetch` segment dropped: `goodfetch::rpc::dispatch` goes to
    `goodfetch.engine.rpc.dispatch`, and a module's default target such as
    `goodfetch_frontier::robots_txt` to
    `goodfetch.engine.goodfetch_frontier.robots_txt`, so a host configures a
    whole area by its prefix. A target with no segment, or one that is no
    identifier, goes to `goodfetch.engine`. Targets are Experimental (ADR
    0132), so the names below `goodfetch.engine` are too; `goodfetch.engine`
    and the pattern are not.
    """
    segments = target.split("::")
    if segments[0] == "goodfetch":
        segments = segments[1:]
    if not segments or not all(segment.isidentifier() for segment in segments):
        return ENGINE_LOGGER
    return ".".join((ENGINE_LOGGER, *segments))


def log_engine_event(params: object) -> None:
    """Logs one of the engine's log events, a `$/log` notification's params.

    At its level, with `source`, `target`, `fields` and `dropped` as the
    record's attributes. The event is built as its model without being
    validated (STYLE.md): one this SDK cannot log, such as one at a level it
    does not know, or one a host's filter raises on, is lost alone, and the
    reading task, which every call of the Connection waits on, goes on.
    """
    members = object_members(params)
    if members is None:
        return
    try:
        event = result_of(LogEvent, members)
        level = _LEVELS.get(event.level)
        if level is None:
            # `off`, which no event carries, or a level this SDK does not know.
            return
        logging.getLogger(engine_logger_name(event.target)).log(
            level,
            "%s",
            event.message,
            extra={
                "source": "engine",
                "target": event.target,
                "fields": event.fields or {},
                "dropped": event.dropped or 0,
            },
        )
    except Exception:
        return
