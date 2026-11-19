"""A program that configures no logging, connects, and closes (ADR 0078).

It runs one of the three ways its second argument names, `asyncio`, `trio`
or `sync`, then records into the file its first argument names each
`goodfetch` logger as the SDK left it: its level, its handlers and whether
it propagates. It writes nothing to stdout or stderr itself, so anything
there is the SDK's. TypeScript's `tests/fixtures/unlogged.ts` is its twin.
"""

from __future__ import annotations

import json
import logging
import os
import sys
from pathlib import Path

import anyio


async def run_async() -> None:
    """Connects with the async API and closes."""
    from goodfetch_collie import AsyncCollie

    async with AsyncCollie.connect(env=os.environ):
        pass


def run_sync() -> None:
    """Connects with the sync API and closes."""
    from goodfetch_collie import Collie

    with Collie.connect(env=os.environ):
        pass


def goodfetch_loggers() -> dict[str, object]:
    """Each `goodfetch` logger that exists: its level, handlers and propagation."""
    return {
        name: [
            logger.level,
            [type(handler).__name__ for handler in logger.handlers],
            logger.propagate,
        ]
        for name, logger in sorted(logging.root.manager.loggerDict.items())
        if isinstance(logger, logging.Logger)
        and (name == "goodfetch" or name.startswith("goodfetch."))
    }


def main(out: Path, way: str) -> None:
    """Connects and closes `way`, then records the `goodfetch` loggers into `out`."""
    if way == "sync":
        run_sync()
    else:
        anyio.run(run_async, backend=way)
    out.write_text(json.dumps(goodfetch_loggers()), "utf-8")


if __name__ == "__main__":
    main(Path(sys.argv[1]), sys.argv[2])
