"""The tracer program (#1218): one Run started and finished, then its Connection closed.

The unit tests run it on the Scripted engine over the in-memory transport,
the integration tests on an Attached engine of the executable of the same
commit (ADR 0069): the same code on both, asyncio, Trio and the sync API.
TypeScript's `tests/programs/tracer.ts` is its twin.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from goodfetch_collie import AsyncCollie, RunReport, StartedAs


@dataclass(frozen=True)
class Traced:
    """What the tracer saw."""

    started_as: StartedAs
    run_id: str
    report: RunReport


async def start_and_finish(collie: AsyncCollie) -> Traced:
    """Starts a Run on `collie`, finishes it, then closes the Connection."""
    run = await collie.start()
    report = await run.finish()
    await collie.close()
    return Traced(started_as=run.started_as, run_id=run.id, report=report)
