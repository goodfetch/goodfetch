"""The host's logging (ADR 0078): the SDK's logger names, and the engine's logs.

Every logger sits under `goodfetch`, which the package gives a `NullHandler`
and nothing else; no handler, level or root logger is touched. The names
below are SDK API under semver, listed in the SDK's manifest; message text
is not. The level of the engine's logs a Connection receives follows the
effective level of `goodfetch.engine`, and each `$/log` event goes to a
`goodfetch.engine.<area>` logger at its level. TypeScript's `logger.ts` is
its twin.
"""

from __future__ import annotations

import logging
from typing import Final

from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.generated.models import LogLevel

CLIENT_LOGGER: Final = "goodfetch.client"
"""The SDK's runtime: the engine's stderr lines and a refused log level, at warning."""

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

    `goodfetch.engine.<area>`, where the area is the target's second path
    segment, its first after the crate: `goodfetch::rpc::dispatch` goes to
    `goodfetch.engine.rpc`. A target with no such segment goes to
    `goodfetch.engine`. Targets are Experimental (ADR 0132), so the areas
    are too; `goodfetch.engine` and the pattern are not.
    """
    segments = target.split("::")
    area = segments[1] if len(segments) > 1 else ""
    if not area or not area.isidentifier():
        return ENGINE_LOGGER
    return f"{ENGINE_LOGGER}.{area}"


def log_engine_event(params: object) -> None:
    """Logs one of the engine's log events, a `$/log` notification's params.

    At its level, with `source`, `target`, `fields` and `dropped` as the
    record's attributes. Params that are no log event, or a host's filter
    that raises, lose the event alone: engine messages are not validated
    (STYLE.md), and the Connection goes on.
    """
    event = object_members(params)
    if event is None:
        return
    name = event.get("level")
    try:
        level = _LEVELS[LogLevel(name)] if isinstance(name, str) else None
    except (ValueError, KeyError):
        # `off`, or a level this SDK does not know.
        return
    target = event.get("target")
    message = event.get("message")
    fields = object_members(event.get("fields")) or {}
    dropped = event.get("dropped")
    if level is None or not isinstance(target, str) or not isinstance(message, str):
        return
    extra = {
        "source": "engine",
        "target": target,
        "fields": fields,
        "dropped": dropped
        if isinstance(dropped, int) and not isinstance(dropped, bool)
        else 0,
    }
    try:
        logging.getLogger(engine_logger_name(target)).log(
            level, "%s", message, extra=extra
        )
    except Exception:
        # A host's filter raised: the event is lost, and the reading task,
        # which every call of the Connection waits on, goes on.
        return
