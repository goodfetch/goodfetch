"""A program that uses Foxhound as an installed platform wheel, one of three ways.

It connects with no engine-path variable of Foxhound's, starts a Run,
finishes it and closes, and prints, as a JSON object, the engine it reached
and the Run's lifecycle. Its argument names the way: `asyncio`, `trio` or
`sync`. TypeScript's `tests/fixtures/installed.ts` of Foxhound's package is
its twin.
"""

from __future__ import annotations

import json
import sys
from typing import TYPE_CHECKING

import anyio
from goodfetch_foxhound import AsyncFoxhound, Foxhound

if TYPE_CHECKING:
    from goodfetch_collie_client.connection import EngineInfo
    from goodfetch_collie_client.generated.models import RunReport


async def finish_async() -> tuple[EngineInfo, RunReport]:
    """Starts and finishes a Run on the event loop."""
    foxhound = await AsyncFoxhound.connect()
    engine = foxhound.engine
    run = await foxhound.start()
    report = await run.finish()
    await foxhound.close()
    return engine, report


def finish_sync() -> tuple[EngineInfo, RunReport]:
    """Starts and finishes a Run without async."""
    with Foxhound.connect() as foxhound:
        run = foxhound.start()
        return foxhound.engine, run.finish()


def main(way: str) -> None:
    """Finishes a Run `way` and prints what it reached."""
    if way == "sync":
        engine, report = finish_sync()
    else:
        engine, report = anyio.run(finish_async, backend=way)
    json.dump(
        {
            "kind": engine.kind,
            "product": engine.product,
            "engine_path": engine.engine_path,
            "lifecycle": report.lifecycle,
        },
        sys.stdout,
    )


if __name__ == "__main__":
    main(sys.argv[1])
