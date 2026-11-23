"""A program that connects with no `env` and closes, one of three ways.

`connect()` then passes the process environment (ADR 0083), which the test
that starts this program gives it, so no test patches its own. It writes
its own environment as a JSON object into the file its first argument
names, for the test to compare with the one its engine was started with.
Its second argument names the way: `asyncio`, `trio` or `sync`.
TypeScript's `tests/fixtures/environment.ts` is its twin.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

import anyio
from goodfetch_collie import AsyncCollie, Collie


async def connect_async() -> None:
    """Connects on the event loop and closes."""
    collie = await AsyncCollie.connect()
    await collie.close()


def main(out: Path, way: str) -> None:
    """Connects and closes `way`, then writes the process environment into `out`."""
    if way == "sync":
        Collie.connect().close()
    else:
        anyio.run(connect_async, backend=way)
    out.write_text(json.dumps(dict(os.environ)), "utf-8")


if __name__ == "__main__":
    main(Path(sys.argv[1]), sys.argv[2])
