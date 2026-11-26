"""Foxhound's Python package finds the executable its platform wheel holds (ADR 0090).

The package's folder, copied as a platform wheel installs it, with the
Scripted engine under Foxhound's name in its `bin`, serves a program that
sets no engine-path variable of Foxhound's, three ways. The wheels built and
installed by `uv` and `pip` wait for issue 3647. TypeScript's
`platform-package.test.ts` is its twin.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

import goodfetch_foxhound
import pytest
from goodfetch_test_support import Product, TestEnv, scripted_engine_env

PROGRAM = (
    Path(__file__).parents[3] / "collie" / "tests" / "programs"
) / "foxhound_installed.py"
"""The program that connects with no engine-path variable of Foxhound's."""

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long the program may take to connect, finish its Run and close."""


def installed_wheel(site: Path, engine: Path) -> Path:
    """The executable of Foxhound's platform wheel installed in `site`, `engine`'s copy.

    The wheel's package is the sources' own, and its dependencies are the
    environment's, which the program's `PYTHONPATH` leaves behind `site`.
    """
    package = site / goodfetch_foxhound.__name__
    shutil.copytree(
        Path(goodfetch_foxhound.__file__).parent,
        package,
        ignore=shutil.ignore_patterns("__pycache__", "bin"),
    )
    suffix = ".exe" if sys.platform == "win32" else ""
    executable = package / "bin" / f"goodfetch-foxhound{suffix}"
    executable.parent.mkdir()
    shutil.copy2(engine, executable)
    return executable


@pytest.mark.covers("story:11-1001", "story:11-1007")
@pytest.mark.parametrize("way", ["asyncio", "trio", "sync"])
def test_starts_the_executable_its_platform_wheel_holds(
    way: str, test_env: TestEnv
) -> None:
    """Whatever Collie's engine-path variable names."""
    # The Scripted engine's map names its engine for Collie alone.
    env = scripted_engine_env(
        test_env, Product.COLLIE, "sdk-python/run/foxhound-started-and-finished.json"
    )
    site = test_env.folder.resolve() / "site"
    engine = Path(env[Product.COLLIE.engine_path_variable])
    executable = installed_wheel(site, engine)

    program = subprocess.run(
        [sys.executable, str(PROGRAM), way],
        capture_output=True,
        text=True,
        cwd=test_env.folder,
        env={
            **env,
            "PYTHONPATH": str(site),
            Product.COLLIE.engine_path_variable: "/nonexistent/goodfetch-collie",
        },
        timeout=PROGRAM_DEADLINE_SECONDS,
        check=False,
    )

    assert program.returncode == 0, program.stderr
    assert json.loads(program.stdout) == {
        "kind": "attached",
        "product": "foxhound",
        "engine_path": str(executable),
        "lifecycle": "finished",
    }
