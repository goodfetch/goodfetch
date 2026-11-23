"""A five-word edit ports a Collie program to Foxhound unchanged otherwise (ADR 0097).

The edit changes the installed package, the import, the connection type,
the executable and the engine-path variable (ADR 0090), and nothing else:
the ported program's page code, settings, commands, `--json` output and
exit codes stay, so both programs print the same thing, each on its own
Product's executable and in a home of its own.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest
from goodfetch_test_support import ClientCaseRecord, Product, TestEnv

PROGRAM = Path(__file__).parents[1] / "programs" / "collie_ported.py"

PROGRAM_DEADLINE_SECONDS = 120.0

FIVE_WORDS = {
    "installed package": ("goodfetch-collie", "goodfetch-foxhound"),
    "import": ("goodfetch_collie", "goodfetch_foxhound"),
    "connection type": ("Collie", "Foxhound"),
    "executable": ("goodfetch-collie", "goodfetch-foxhound"),
    "engine-path variable": (
        Product.COLLIE.engine_path_variable,
        Product.FOXHOUND.engine_path_variable,
    ),
}
"""Each word the move to Foxhound changes, Collie's and Foxhound's.

In Python the installed package and the executable share their name."""


def whole_word(word: str) -> re.Pattern[str]:
    """`word` where no letter, digit, `_` or `-` touches it."""
    return re.compile(rf"(?<![\w-]){re.escape(word)}(?![\w-])")


def ported(program: str) -> str:
    """`program` with each of Collie's five words replaced by Foxhound's."""
    for collie, foxhound in FIVE_WORDS.values():
        program = whole_word(collie).sub(foxhound, program)
    return program


def played(
    program: Path, test_env: TestEnv, home: Path
) -> subprocess.CompletedProcess[str]:
    """`program` run in `home` with both executables on `PATH`.

    Both engine-path variables are set, as in an environment that serves
    both Products, so each program reads its own.
    """
    tools = [test_env.tools.path(each.tool) for each in Product]
    path = os.pathsep.join(
        [*(str(tool.parent) for tool in tools), test_env.env.get("PATH", os.defpath)]
    )
    env = {
        **test_env.env,
        "PATH": path,
        "GOODFETCH_HOME": str(home),
        **{
            each.engine_path_variable: str(tool)
            for each, tool in zip(Product, tools, strict=True)
        },
    }
    return subprocess.run(
        [sys.executable, str(program)],
        capture_output=True,
        text=True,
        cwd=test_env.folder,
        env=env,
        timeout=PROGRAM_DEADLINE_SECONDS,
        check=False,
    )


@pytest.mark.client_case(id="five-word-port", mode="attached")
def test_a_five_word_edit_ports_a_collie_program_that_then_prints_the_same(
    test_env: TestEnv, client_case: ClientCaseRecord
) -> None:
    """The ported program prints on Foxhound what the original prints on Collie."""
    program = PROGRAM.read_text("utf-8")
    port = ported(program)
    port_file = test_env.folder / "foxhound_ported.py"
    port_file.write_text(port, "utf-8")

    collie = played(PROGRAM, test_env, test_env.home)
    foxhound = played(port_file, test_env, test_env.folder / "foxhound home")

    with client_case.check("the program names each of the five words, Collie's"):
        assert [
            role
            for role, (word, _) in FIVE_WORDS.items()
            if not whole_word(word).search(program)
        ] == []
    with client_case.check("the port names none of Collie's"):
        assert [
            word for word, _ in FIVE_WORDS.values() if whole_word(word).search(port)
        ] == []
    with client_case.check("both programs exited 0"):
        assert (collie.returncode, foxhound.returncode) == (0, 0), (
            collie.stderr + foxhound.stderr
        )
    seen = json.loads(collie.stdout)
    client_case.message(seen)
    with client_case.check(
        "Collie's program reached its variable's engine, finished its Run and"
        " was refused a Run no index lists with exit code 3"
    ):
        assert (
            seen["reached_the_variables_engine"],
            seen["report"],
            seen["refused"][0],
        ) == (True, ["ported", "finished"], 3)
    with client_case.check(
        "the port printed the same results, `--json` output and exit codes"
    ):
        assert foxhound.stdout == collie.stdout
