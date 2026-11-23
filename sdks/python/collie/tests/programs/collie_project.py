"""A program that connects from its working directory, one of three ways.

The test that starts it chooses its working directory, so no test moves its
own. With `found` as its third argument it passes no `project`, so
`connect()` reads the project file found upward from there, as the CLI
finds it; with `none` it passes `project=None` and reads none (ADR 0083).
It explains `timeout.navigation`, starts a Run and finishes it, against a
script that expects exactly the project it carries, then writes the file
its engine reports and the Run's lifecycle as a JSON object into the file
its first argument names. Its second argument names the way: `asyncio`,
`trio` or `sync`. TypeScript's `tests/fixtures/project.ts` is its twin.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import anyio
from goodfetch_collie import AsyncCollie, Collie
from goodfetch_collie_client.handle import NotGiven, not_given

KEY = "timeout.navigation"
"""The setting the program explains."""


async def played_async(project: NotGiven | None) -> tuple[str | None, str]:
    """Plays on the event loop: the file the engine reports, the Run's lifecycle."""
    collie = await AsyncCollie.connect(project=project)
    await collie.explain_settings([KEY])
    report = await (await collie.start()).finish()
    await collie.close()
    return collie.engine.project_file, report.lifecycle


def played_sync(project: NotGiven | None) -> tuple[str | None, str]:
    """Plays through the sync API: the file the engine reports, the Run's lifecycle."""
    collie = Collie.connect(project=project)
    collie.explain_settings([KEY])
    report = collie.start().finish()
    collie.close()
    return collie.engine.project_file, report.lifecycle


def main(out: Path, way: str, chosen: str) -> None:
    """Plays `way` with the project file `chosen` picks, and writes what it saw."""
    project = not_given if chosen == "found" else None
    if way == "sync":
        project_file, lifecycle = played_sync(project)
    else:
        project_file, lifecycle = anyio.run(played_async, project, backend=way)
    out.write_text(
        json.dumps({"project_file": project_file, "lifecycle": lifecycle}), "utf-8"
    )


if __name__ == "__main__":
    main(Path(sys.argv[1]), sys.argv[2], sys.argv[3])
