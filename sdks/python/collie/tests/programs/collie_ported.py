# /// script
# dependencies = ["goodfetch-collie"]
# ///
"""A Collie program that a five-word edit ports to Foxhound (ADR 0097).

It connects to the engine its engine-path variable names, under a setting,
starts and finishes a Run, then runs its executable's `run report` and
`run list` and a `run report` that fails, and prints, as JSON, what each
gave: the program's whole output, which its Foxhound port must print
unchanged. `test_five_word_port.py` makes the port and runs both.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from typing import TypeAlias

from goodfetch_collie import Collie

EXECUTABLE = "goodfetch-collie"
"""The executable this program's commands run, from `PATH`."""


Json: TypeAlias = "dict[str, Json] | list[Json] | str | int | float | bool | None"
"""A parsed JSON value."""


def untimed(value: Json) -> Json:
    """`value` without the times a Run started and ended, which no two runs share."""
    if isinstance(value, dict):
        return {
            key: untimed(item)
            for key, item in value.items()
            if key not in {"started_at", "ended_at"}
        }
    if isinstance(value, list):
        return [untimed(item) for item in value]
    return value


def command(*arguments: str) -> tuple[int, Json]:
    """The exit code and untimed `--json` output of a command of the executable."""
    ran = subprocess.run(
        [EXECUTABLE, *arguments, "--json"],
        capture_output=True,
        text=True,
        check=False,
    )
    parsed: Json = json.loads(ran.stdout)
    return ran.returncode, untimed(parsed)


def main() -> None:
    """Plays the program and prints what it saw."""
    engine_path = os.environ["GOODFETCH_COLLIE_ENGINE_PATH"]
    with Collie.connect(
        engine="attached", settings={"connection": {"max_pending_calls": 8}}
    ) as connection:
        reached = connection.engine.engine_path == engine_path
        run = connection.start(run_id="ported")
        report = run.finish()
    reported_code, reported = command("run", "report", "ported")
    listed_code, listed = command("run", "list")
    refused_code, refused = command("run", "report", "no-such-run")
    json.dump(
        {
            "reached_the_variables_engine": reached,
            "report": [report.run_id.root, report.lifecycle],
            "run_report": [reported_code, reported],
            "run_list": [listed_code, listed],
            "refused": [refused_code, refused],
        },
        sys.stdout,
        indent=2,
        sort_keys=True,
    )


if __name__ == "__main__":
    main()
