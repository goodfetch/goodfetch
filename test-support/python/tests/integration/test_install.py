"""Seam B (spec #326): wheels install into a fresh environment (ADR 0069).

Fixture wheels of the Collie client and both Products install with uv and
pip, offline from a folder of the wheels alone, on the host (glibc on Linux)
and in the pinned Alpine image (musl), and a Product's `tests/integration`
runs there unchanged. The Release suite alone collects these tests
(`conftest.py`), as it alone runs every installed-package run (TESTING.md,
Installed packages).
"""

import os
import shutil
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path
from typing import TypeAlias

import pytest
from goodfetch_test_support import TestEnv
from goodfetch_test_support.install import (
    SYSTEMS,
    CommandOutput,
    Installer,
    System,
    install_wheel,
)

FIXTURES = Path(__file__).parent / "fixtures" / "install"
"""The fixture projects: the Collie client and the two Products."""

BUILD_DEADLINE_SECONDS = 120.0
"""How long one wheel's build may take."""

BuildWheel: TypeAlias = Callable[[str, str | None], Path]


@pytest.fixture
def build_wheel(test_env: TestEnv) -> BuildWheel:
    """Builds a fixture project's wheel with uv, offline, in the test's folder.

    It takes the project's folder name under `fixtures/install`, and a file
    name the wheel leaves out (uv_build's `wheel-exclude`), or `None`.
    """

    def build(project: str, exclude: str | None) -> Path:
        source = test_env.folder / "fixture projects" / project
        shutil.copytree(FIXTURES / project, source)
        if exclude is not None:
            with (source / "pyproject.toml").open("a", encoding="utf-8") as manifest:
                manifest.write(
                    f'\n[tool.uv.build-backend]\nwheel-exclude = ["{exclude}"]\n'
                )
        out = test_env.folder / "fixture wheels" / project
        uv = shutil.which("uv", path=os.environ.get("PATH"))
        assert uv is not None, "uv is not on PATH"
        built = subprocess.run(
            [
                uv,
                "build",
                "--wheel",
                "--offline",
                "--no-config",
                "--python",
                sys.executable,
                "--out-dir",
                str(out),
                str(source),
            ],
            capture_output=True,
            env={
                **test_env.env,
                "PATH": os.environ.get("PATH", ""),
                "UV_CACHE_DIR": str(test_env.folder / "build cache"),
                "UV_PYTHON_DOWNLOADS": "never",
            },
            timeout=BUILD_DEADLINE_SECONDS,
            check=False,
        )
        assert built.returncode == 0, built.stderr.decode(errors="replace")
        [wheel] = out.glob("*.whl")
        return wheel

    return build


on_every_system = pytest.mark.parametrize("system", SYSTEMS, ids=str)
with_every_installer = pytest.mark.parametrize("installer", list(Installer), ids=str)


@on_every_system
@with_every_installer
def test_a_product_s_integration_tests_pass_against_its_installed_wheel(
    test_env: TestEnv, build_wheel: BuildWheel, installer: Installer, system: System
) -> None:
    """Collie's wheel and the client it pins install, and its tests pass."""
    wheels = [build_wheel("collie-client", None), build_wheel("collie", None)]

    environment = install_wheel(test_env, wheels, installer=installer, system=system)

    assert environment.folder.name == f"{installer} {system} café"
    assert environment.install.exit == 0, environment.install.stderr
    tests = environment.run_tests(FIXTURES / "collie" / "tests" / "integration")
    assert tests.exit == 0, tests.stdout + tests.stderr
    assert "2 passed" in tests.stdout


@on_every_system
@with_every_installer
def test_both_products_install_into_one_environment_and_both_executables_run(
    test_env: TestEnv, build_wheel: BuildWheel, installer: Installer, system: System
) -> None:
    """Both Products pin the same client, and each executable runs."""
    wheels = [
        build_wheel("collie-client", None),
        build_wheel("collie", None),
        build_wheel("foxhound", None),
    ]

    environment = install_wheel(test_env, wheels, installer=installer, system=system)

    assert environment.install.exit == 0, environment.install.stderr
    assert environment.run("goodfetch-collie") == CommandOutput(
        0, "goodfetch-collie 0.2.0\n", ""
    )
    assert environment.run("goodfetch-foxhound") == CommandOutput(
        0, "goodfetch-foxhound 0.2.0\n", ""
    )


@on_every_system
@with_every_installer
def test_a_file_the_wheel_leaves_out_fails_the_integration_tests(
    test_env: TestEnv, build_wheel: BuildWheel, installer: Installer, system: System
) -> None:
    """A client wheel without `greeting.py` installs, and Collie's tests fail."""
    wheels = [build_wheel("collie-client", "greeting.py"), build_wheel("collie", None)]

    environment = install_wheel(test_env, wheels, installer=installer, system=system)

    assert environment.install.exit == 0, environment.install.stderr
    tests = environment.run_tests(FIXTURES / "collie" / "tests" / "integration")
    assert tests.exit != 0
    assert "No module named 'goodfetch_collie_client.greeting'" in tests.stdout


@on_every_system
@with_every_installer
def test_a_pinned_wheel_missing_from_the_folder_fails_the_install(
    test_env: TestEnv, build_wheel: BuildWheel, installer: Installer, system: System
) -> None:
    """With no index, the install cannot fetch the client Collie pins."""
    environment = install_wheel(
        test_env, [build_wheel("collie", None)], installer=installer, system=system
    )

    assert environment.install.exit != 0
    assert "goodfetch-collie-client" in environment.install.stdout + (
        environment.install.stderr
    )
