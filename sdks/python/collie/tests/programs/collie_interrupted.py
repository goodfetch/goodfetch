"""A host program that Ctrl+C interrupts while its Run runs (ADRs 0078 and 0093).

It connects one of the three ways its third argument names, `asyncio`,
`trio` or `sync`, starts the Run `nightly`, creates the file its second
argument names once the Run runs, and waits. Ctrl+C's `KeyboardInterrupt`
unwinds its `with` blocks, which close the Run, stopping it with a
Checkpoint, then the Connection, as the SDK's docs advise; it then writes
into the file its first argument names whether it was interrupted and what
its `goodfetch` loggers heard at warning, and exits 0. It writes nothing to
stdout or stderr itself, so anything there is the SDK's. TypeScript's
`tests/fixtures/interrupted.ts` is its twin.
"""

from __future__ import annotations

import json
import logging
import os
import sys
import threading
import warnings
from pathlib import Path
from typing import TYPE_CHECKING

import anyio

if TYPE_CHECKING:
    from collections.abc import Mapping


class Heard(logging.Handler):
    """Keeps each record's message."""

    def __init__(self) -> None:
        """Starts with nothing heard."""
        super().__init__(logging.WARNING)
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        """Keeps the record's message."""
        self.messages.append(record.getMessage())


async def run_async(env: Mapping[str, str], running: Path) -> None:
    """Connects with the async API, starts the Run and waits inside both blocks."""
    from goodfetch_collie import AsyncCollie

    async with (
        AsyncCollie.connect(env=env) as collie,
        await collie.start(run_id="nightly", tags=["shop"]),
    ):
        await anyio.Path(running).touch()
        await anyio.sleep_forever()


def run_sync(env: Mapping[str, str], running: Path) -> None:
    """Connects with the sync API, starts the Run and waits inside both blocks."""
    from goodfetch_collie import Collie

    with (
        Collie.connect(env=env) as collie,
        collie.start(run_id="nightly", tags=["shop"]),
    ):
        running.touch()
        threading.Event().wait()


def main(out: Path, running: Path, way: str) -> None:
    """Runs `way` until Ctrl+C, then records what it saw into `out`."""
    heard = Heard()
    logging.getLogger("goodfetch").addHandler(heard)
    # The catalog's marker warnings (ADR 0030) reach the host's own warning
    # channel, which this host silences, so stderr holds only the SDK's output.
    warnings.simplefilter("ignore")
    env = dict(os.environ)
    interrupted = False
    try:
        if way == "sync":
            run_sync(env, running)
        else:
            anyio.run(run_async, env, running, backend=way)
    except KeyboardInterrupt:
        interrupted = True
    out.write_text(
        json.dumps({"interrupted": interrupted, "warnings": heard.messages}), "utf-8"
    )


if __name__ == "__main__":
    main(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3])
