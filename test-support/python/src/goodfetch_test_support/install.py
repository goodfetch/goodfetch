"""The PyPI side of the install harness (ADR 0069; TESTING.md, Installed packages).

`install_wheel` installs wheels into a fresh virtual environment with uv or
pip, offline from a folder of the wheels alone (`--no-index`), as a user's
install from PyPI resolves them, and gives the environment to run its console
scripts and a package's `tests/integration` in. The environment is on the
host, glibc on Linux, or in the pinned image of uv on Alpine Linux, musl
(`ci/alpine/image.toml`), run with no network through Docker or Podman.
`install_lowest_direct` installs an adapter's wheel the same way with each of
its requirements at the lowest version the range allows, for ADR 0070's
lowest-version runs.

Every project lives in the test's folder under a name that holds a space and
a non-ASCII letter, with a home, caches and no configuration of its own. The
install's output is returned, success or not, for the test to judge. A tool
the run needs that is missing fails with its own error, never a skip
(ADR 0069).
"""

import email
import os
import shutil
import subprocess
import sys
import tomllib
import uuid
import zipfile
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from enum import StrEnum
from importlib.metadata import Distribution, PackageNotFoundError, distribution
from importlib.util import find_spec
from pathlib import Path

from packaging.requirements import Requirement
from packaging.utils import canonicalize_name, parse_wheel_filename

from goodfetch_test_support.environment import TestEnv

IMAGE_FILE = Path(__file__).resolve().parents[4] / "ci" / "alpine" / "image.toml"
"""The pin of the Alpine image, which `just fetch` pulls."""

COMMAND_DEADLINE_SECONDS = 300.0
"""How long one install, script or test run may take."""

_ENGINE_DEADLINE_SECONDS = 60.0
"""How long the container engine may take to answer `info` or `image inspect`."""

_CONTAINER_PYTHON = "/usr/local/bin/python3"
"""The image's Python, which the environments in it are made from."""


class Installer(StrEnum):
    """The tool a user installs a Goodfetch wheel with."""

    UV = "uv"
    PIP = "pip"


class System(StrEnum):
    """Where the environment lives."""

    HOST = "host"
    """The running Python's own system: glibc on Linux."""
    ALPINE = "alpine"
    """The pinned image of uv on Alpine Linux, musl; Linux hosts only."""


SYSTEMS: tuple[System, ...] = (
    (System.HOST, System.ALPINE) if sys.platform == "linux" else (System.HOST,)
)
"""The systems this host installs on: Alpine through a container on Linux."""


class ToolMissing(Exception):
    """A tool the install needs is not on this host."""


class ContainerEngineMissing(Exception):
    """Neither Docker nor Podman answers, and an Alpine install needs one."""


class ImageMissing(Exception):
    """The pinned Alpine image is not pulled; the message names `just fetch`."""


@dataclass(frozen=True)
class CommandOutput:
    """What a command left: its exit status and output."""

    exit: int
    """Its exit status."""
    stdout: str
    """Its standard output."""
    stderr: str
    """Its standard error."""


@dataclass(frozen=True)
class _Container:
    """Runs commands in the Alpine image, with the project's folder mounted."""

    engine: str
    image: str
    folder: Path

    def wrap(self, args: Sequence[str], env: Mapping[str, str]) -> list[str]:
        user = (
            ["--userns=keep-id"]
            if Path(self.engine).name == "podman"
            else [f"--user={os.getuid()}:{os.getgid()}"]
        )
        variables = [f"--env={name}={value}" for name, value in env.items()]
        return [
            self.engine,
            "run",
            "--rm",
            f"--name={self.name}",
            "--network=none",
            *user,
            f"--volume={self.folder}:{self.folder}",
            f"--workdir={self.folder}",
            *variables,
            "--entrypoint",
            args[0],
            self.image,
            *args[1:],
        ]

    @property
    def name(self) -> str:
        return f"goodfetch-install-{uuid.uuid5(uuid.NAMESPACE_URL, str(self.folder))}"


@dataclass(frozen=True)
class InstalledEnvironment:
    """A fresh environment the wheels were installed into."""

    folder: Path
    """The project's folder, whose name holds a space and a non-ASCII letter."""
    install: CommandOutput
    """The install's output; the test judges its exit status."""
    _venv: Path
    _env: Mapping[str, str]
    _container: _Container | None

    def run(self, program: str, *args: str) -> CommandOutput:
        """Runs a console script of the environment, as a user does.

        Args:
            program: The script's name, which the installed wheels declare.
            args: Its arguments.
        """
        return self._run([str(_scripts(self._venv) / program), *args])

    def run_tests(self, tests: Path) -> CommandOutput:
        """Runs a package's `tests/integration` unchanged against the environment.

        The tests are copied into the project and run with the environment's
        Python and the running pytest, which is not installed there: only
        the wheels are.

        Args:
            tests: The folder of the tests.
        """
        copy = self.folder / "tests" / tests.name
        shutil.copytree(tests, copy, ignore=shutil.ignore_patterns("__pycache__"))
        tools = self.folder / "tools"
        if not tools.exists():
            _copy_pytest(tools)
        (self.folder / "pytest.ini").write_text("[pytest]\n", "utf-8")
        return self._run(
            [
                str(_python(self._venv)),
                "-B",
                "-m",
                "pytest",
                "-p",
                "no:cacheprovider",
                "--import-mode=importlib",
                f"--rootdir={self.folder}",
                str(copy),
            ],
            extra={
                "PYTHONPATH": str(tools),
                "PYTEST_DISABLE_PLUGIN_AUTOLOAD": "1",
            },
        )

    def _run(
        self, args: Sequence[str], extra: Mapping[str, str] | None = None
    ) -> CommandOutput:
        return _run(args, {**self._env, **(extra or {})}, self.folder, self._container)


def install_wheel(
    test_env: TestEnv,
    wheels: Iterable[Path],
    *,
    installer: Installer,
    system: System,
) -> InstalledEnvironment:
    """Installs `wheels` by name and version into a new environment.

    The wheels are copied into the project's `wheels` folder, the only place
    the install looks (`--no-index --find-links`), and each is asked for as
    `name==version`, so a dependency they pin must be among them.

    Args:
        test_env: The test's setup, in whose folder the project lives.
        wheels: The wheel files.
        installer: uv (`uv pip install`) or pip (`pip --python <venv> install`).
        system: The host, or the Alpine image on a Linux host.

    Raises:
        ToolMissing: uv, or pip for a pip install on the host, is missing.
        ContainerEngineMissing: An Alpine install finds no Docker or Podman.
        ImageMissing: The Alpine image is not pulled.
    """
    wheels = list(wheels)
    return _install(
        test_env,
        f"{installer} {system} café",
        wheels,
        [_pinned(wheel) for wheel in wheels],
        installer=installer,
        system=system,
    )


def install_lowest_direct(
    test_env: TestEnv,
    wheel: Path,
    available: Iterable[Path],
    *,
    system: System,
) -> InstalledEnvironment:
    """Installs `wheel` with each of its requirements at its range's lowest version.

    ADR 0070's lowest-version runs: uv resolves with `--resolution
    lowest-direct`, which takes the lowest version a direct requirement
    allows and the highest of everything else. The wheel's own requirements
    are passed as direct ones beside `name==version`, since uv would take
    them as transitive. The install looks only in the project's `wheels`
    folder, where `wheel` and `available` are copied, so every version the
    ranges could pick is among `available`.

    Args:
        test_env: The test's setup, in whose folder the project lives.
        wheel: The wheel under test, an adapter's.
        available: The wheels its requirements may resolve to, a framework's
            versions among them.
        system: The host, or the Alpine image on a Linux host.

    Raises:
        ToolMissing: uv is missing.
        ContainerEngineMissing: An Alpine install finds no Docker or Podman.
        ImageMissing: The Alpine image is not pulled.
    """
    return _install(
        test_env,
        f"uv lowest-direct {system} café",
        [wheel, *available],
        [_pinned(wheel), *_requires(wheel)],
        installer=Installer.UV,
        system=system,
        options=["--resolution", "lowest-direct"],
    )


def _pinned(wheel: Path) -> str:
    name, version, _, _ = parse_wheel_filename(wheel.name)
    return f"{name}=={version}"


def _requires(wheel: Path) -> list[str]:
    """The `Requires-Dist` lines of the wheel's metadata."""
    with zipfile.ZipFile(wheel) as archive:
        [metadata] = [
            name
            for name in archive.namelist()
            if name.count("/") == 1 and name.endswith(".dist-info/METADATA")
        ]
        message = email.message_from_bytes(archive.read(metadata))
    return [str(line) for line in message.get_all("Requires-Dist") or ()]


def _install(
    test_env: TestEnv,
    name: str,
    wheels: Iterable[Path],
    requirements: Sequence[str],
    *,
    installer: Installer,
    system: System,
    options: Sequence[str] = (),
) -> InstalledEnvironment:
    folder = test_env.folder / name
    links = folder / "wheels"
    links.mkdir(parents=True)
    for wheel in wheels:
        shutil.copy2(wheel, links / wheel.name)
    home = folder / "home"
    home.mkdir()
    venv = folder / "venv"
    env = {
        "HOME": str(home),
        "UV_CACHE_DIR": str(folder / "cache"),
        "UV_PYTHON_DOWNLOADS": "never",
        "UV_NO_CONFIG": "1",
        "NO_COLOR": "1",
        "PYTHONNOUSERSITE": "1",
    }
    if system is System.HOST:
        container = None
        env = {**test_env.env, **env, "PATH": os.environ.get("PATH", "")}
        uv = _tool("uv")
        python = sys.executable
        pip = [sys.executable, "-m", "pip"]
        if installer is Installer.PIP and find_spec("pip") is None:
            message = "pip is not installed beside the running Python"
            raise ToolMissing(message)
    else:
        container = _container(folder.resolve())
        folder = container.folder
        links, home, venv = (folder / path.name for path in (links, home, venv))
        env = {**env, "HOME": str(home), "UV_CACHE_DIR": str(folder / "cache")}
        uv = "uv"
        python = _CONTAINER_PYTHON
        pip = [_CONTAINER_PYTHON, "-m", "pip"]

    created = _run(
        [uv, "venv", "--python", python, str(venv)],
        env,
        folder,
        container,
    )
    if created.exit != 0:
        return InstalledEnvironment(folder, created, venv, env, container)
    target = str(_python(venv))
    if installer is Installer.UV:
        command = [uv, "pip", "install", "--python", target]
    else:
        command = [
            *pip,
            "--isolated",
            "--disable-pip-version-check",
            "--no-input",
            "--python",
            target,
            "install",
            "--no-cache-dir",
        ]
    install = _run(
        [
            *command,
            *options,
            "--no-index",
            "--find-links",
            str(links),
            *requirements,
        ],
        env,
        folder,
        container,
    )
    return InstalledEnvironment(folder, install, venv, env, container)


def _tool(name: str) -> str:
    path = shutil.which(name, path=os.environ.get("PATH"))
    if path is None:
        message = f"{name} is not on PATH"
        raise ToolMissing(message)
    return path


def _container(folder: Path) -> _Container:
    engine = next(
        (
            path
            for path in (
                shutil.which(name, path=os.environ.get("PATH"))
                for name in ("docker", "podman")
            )
            if path is not None and _answers(path, "info")
        ),
        None,
    )
    if engine is None:
        message = "neither Docker nor Podman answers, and Alpine installs run in one"
        raise ContainerEngineMissing(message)
    pin = tomllib.loads(IMAGE_FILE.read_text("utf-8"))
    image = f"{pin['repository']}@{pin['digest']}"
    if not _answers(engine, "image", "inspect", image):
        message = f"{image} is not pulled: run `just fetch` (ci/alpine/fetch.sh)"
        raise ImageMissing(message)
    return _Container(engine, image, folder)


def _answers(engine: str, *args: str) -> bool:
    try:
        return (
            subprocess.run(
                [engine, *args],
                capture_output=True,
                timeout=_ENGINE_DEADLINE_SECONDS,
                check=False,
            ).returncode
            == 0
        )
    except subprocess.TimeoutExpired:
        return False


def _run(
    args: Sequence[str],
    env: Mapping[str, str],
    folder: Path,
    container: _Container | None,
) -> CommandOutput:
    command = list(args) if container is None else container.wrap(args, env)
    try:
        done = subprocess.run(
            command,
            capture_output=True,
            cwd=folder,
            env=dict(env) if container is None else None,
            timeout=COMMAND_DEADLINE_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        if container is not None:
            subprocess.run(
                [container.engine, "rm", "--force", container.name],
                capture_output=True,
                timeout=_ENGINE_DEADLINE_SECONDS,
                check=False,
            )
        raise
    return CommandOutput(
        done.returncode,
        done.stdout.decode(errors="replace"),
        done.stderr.decode(errors="replace"),
    )


def _scripts(venv: Path) -> Path:
    return venv / ("Scripts" if sys.platform == "win32" else "bin")


def _python(venv: Path) -> Path:
    return _scripts(venv) / ("python.exe" if sys.platform == "win32" else "python")


def _copy_pytest(into: Path) -> None:
    """Copies the running pytest and what it requires into `into`.

    The fresh environment has only the wheels; its tests import pytest from
    this folder, on `PYTHONPATH`, rather than from an install that could
    reach the network.
    """
    for dist in _requirement_closure("pytest"):
        root = Path(str(dist.locate_file(""))).resolve()
        for file in dist.files or ():
            source = Path(str(dist.locate_file(file))).resolve()
            if "__pycache__" in source.parts or not source.is_relative_to(root):
                continue
            target = into / source.relative_to(root)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)


def _requirement_closure(name: str) -> list[Distribution]:
    found: dict[str, Distribution] = {}
    pending = [name]
    while pending:
        key = canonicalize_name(pending.pop())
        if key in found:
            continue
        try:
            found[key] = distribution(key)
        except PackageNotFoundError:
            message = f"{key}, which the running pytest requires, is not installed"
            raise ToolMissing(message) from None
        for text in found[key].requires or ():
            requirement = Requirement(text)
            if requirement.marker is None or requirement.marker.evaluate({"extra": ""}):
                pending.append(requirement.name)
    return list(found.values())
