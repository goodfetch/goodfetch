"""One test's setup, which `goodfetch-test-env` prepares (ADR 0062).

`TestEnv.open` starts `goodfetch-test-env` from the test tool manifest (with
the Proving ground on a seed and a Product's Daemon, when asked for), reads
its one-line JSON description, checks it against the committed schema, and
records every process the test starts through it. `TestEnv.close` closes
`goodfetch-test-env`'s stdin, which tears the setup down, and fails when a
recorded process is still alive (ADR 0069).
"""

import json
import os
import shutil
import subprocess
from collections.abc import Mapping, Sequence
from datetime import datetime
from enum import StrEnum
from pathlib import Path
from typing import Self, cast

import anyio

from goodfetch_test_support.daemon import Daemon
from goodfetch_test_support.play_script import CORPUS
from goodfetch_test_support.proving_ground import ProvingGround
from goodfetch_test_support.schemas import validate
from goodfetch_test_support.tools import Tool, Tools, read_tools

TEARDOWN_DEADLINE_SECONDS = 10.0
"""How long `goodfetch-test-env` has to tear down once its stdin closes."""


class Product(StrEnum):
    """A Goodfetch Product, whose executable an engine test runs (ADR 0094)."""

    COLLIE = "collie"
    FOXHOUND = "foxhound"

    @property
    def tool(self) -> Tool:
        """The Product's executable in the test tool manifest."""
        return Tool(self.value)

    @property
    def engine_path_variable(self) -> str:
        """The variable its SDK reads the engine's path from (ADR 0090)."""
        return f"GOODFETCH_{self.name}_ENGINE_PATH"


class TestEnvError(Exception):
    """`goodfetch-test-env` failed to start or to tear down."""


class ProcessesAlive(Exception):
    """Processes the test started were still alive after its teardown (ADR 0069)."""

    def __init__(self, commands: Sequence[Sequence[str]]) -> None:
        """Names each live process by its command line."""
        self.commands = [list(command) for command in commands]
        super().__init__(f"processes alive after the test's teardown: {self.commands}")


class TestEnv:
    """One test's folder, home, cleared environment map, seed, start time and tools."""

    __test__ = False

    def __init__(
        self,
        process: subprocess.Popen[bytes],
        description: Mapping[str, object],
        teardown_deadline: float = TEARDOWN_DEADLINE_SECONDS,
    ) -> None:
        """Takes the running `goodfetch-test-env` and its checked description."""
        self._process = process
        self._teardown_deadline = teardown_deadline
        self._started: list[tuple[list[str], subprocess.Popen[bytes]]] = []
        self.folder = Path(cast("str", description["folder"]))
        """The test's own folder, removed when the test ends."""
        self.home = Path(cast("str", description["home"]))
        """`<folder>/home`, the `GOODFETCH_HOME` of everything the test starts."""
        self.env: Mapping[str, str] = dict(cast("dict[str, str]", description["env"]))
        """The cleared environment of every child the test starts."""
        self.seed = cast("int", description["seed"])
        """The test's seed, fixed by its name unless `GOODFETCH_TEST_SEED` is set."""
        self.start_time = datetime.fromisoformat(cast("str", description["start_time"]))
        """The test's start time, in UTC."""
        self.tools = Tools.from_json(description["tools"])
        """The commit's executables, from the test tool manifest."""
        proving_ground = description.get("proving_ground")
        self.proving_ground = (
            None if proving_ground is None else ProvingGround.from_json(proving_ground)
        )
        """The Proving ground started on the seed the test asked for, if any."""
        daemon = description.get("daemon")
        self.daemon = None if daemon is None else Daemon.from_json(daemon)
        """The Daemon started in the test's home, if the test asked for one."""

    @classmethod
    def open(
        cls,
        test_name: str,
        environ: Mapping[str, str],
        *,
        proving_ground_seed: int | None = None,
        daemon: Product | None = None,
        browser: bool = False,
        teardown_deadline: float = TEARDOWN_DEADLINE_SECONDS,
    ) -> Self:
        """Starts `goodfetch-test-env` for the test named `test_name`.

        Args:
            test_name: The test's full name, from which its seed and folder come.
            environ: The runner's environment, which names the tool manifest.
            proving_ground_seed: The seed of a Proving ground to start beside
                the setup, if any.
            daemon: The Product whose Daemon to start in the test's home, if any.
            browser: Whether to lay out the home's Browser cache and reach the
                Proving ground's hosts, for an engine that opens a page.
            teardown_deadline: How many seconds teardown waits for
                `goodfetch-test-env` to end once its stdin closes; 10 unless a
                test sets a shorter one.

        Raises:
            ToolsError: The manifest or its `test_env` entry is missing.
            TestEnvError: `goodfetch-test-env` printed no description.
            SchemaError: The description fails its committed schema.
        """
        executable = read_tools(environ).path(Tool.TEST_ENV)
        proving_ground = (
            []
            if proving_ground_seed is None
            else ["--proving-ground", str(proving_ground_seed)]
        )
        daemon_flag = [] if daemon is None else [f"--daemon={daemon.value}"]
        browser_flag = ["--browser"] if browser else []
        process = subprocess.Popen(
            [
                executable,
                "--test-name",
                test_name,
                *proving_ground,
                *daemon_flag,
                *browser_flag,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            env=dict(environ),
        )
        line = process.stdout.readline() if process.stdout is not None else b""
        try:
            if not line:
                status = process.wait()
                message = (
                    f"goodfetch-test-env exited with {status} before its description"
                )
                raise TestEnvError(message)
            description: object = json.loads(line)
            validate(description, "test-env-description.schema.json")
        except BaseException:
            _stop(process)
            raise
        return cls(process, cast("dict[str, object]", description), teardown_deadline)

    def start(
        self,
        args: Sequence[str | os.PathLike[str]],
        *,
        env: Mapping[str, str] | None = None,
    ) -> subprocess.Popen[bytes]:
        """Starts a process in the test's folder, recorded until teardown.

        Its stdin and stdout are pipes, and its stderr is the test's own.

        Args:
            args: The program and its arguments.
            env: Its environment, the test's cleared map unless given.
        """
        command = [os.fspath(arg) for arg in args]
        process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            cwd=self.folder,
            env=dict(self.env if env is None else env),
        )
        self._started.append((command, process))
        return process

    def close(self, *, failed: bool = False) -> None:
        """Tears the setup down, failing if a process the test started is alive.

        Args:
            failed: Whether the test failed, whose folder then stays for its
                evidence, as a Rust test's does.

        Raises:
            ProcessesAlive: A recorded process or `goodfetch-test-env` was alive.
            TestEnvError: `goodfetch-test-env` failed its teardown.
        """
        alive = [
            command for command, process in self._started if process.poll() is None
        ]
        for _, process in self._started:
            _stop(process)
        if self._process.stdin is not None:
            if failed:
                self._process.stdin.write(b"failed\n")
            self._process.stdin.close()
        try:
            status = self._process.wait(self._teardown_deadline)
        except subprocess.TimeoutExpired:
            alive.append(
                [os.fspath(arg) for arg in cast("list[str]", self._process.args)]
            )
            status = 0
        _stop(self._process)
        if alive:
            raise ProcessesAlive(alive)
        if status != 0:
            message = f"goodfetch-test-env exited with {status} at teardown"
            raise TestEnvError(message)


SCRIPTED_ENGINE_FOLDER = "scripted-engine"
"""The folder under a home that holds the Scripted engine's script and record."""


def scripted_engine_env(
    test_env: TestEnv, product: Product, script: str, home: Path | None = None
) -> Mapping[str, str]:
    """The map under which `product`'s SDK starts the Scripted engine playing `script`.

    The corpus's script `script`, such as `"sdk-python/run/started-and-finished.json"`,
    is installed as `scripted-engine/script.json` under `home`, which the
    map's `GOODFETCH_HOME` names, and `goodfetch-scripted-engine` writes its
    record beside it. Each Connection a test opens at once takes a home of
    its own. TypeScript's `scriptedEngineEnv` is its twin.

    Args:
        test_env: The test's setup, whose map and tools it starts from.
        product: The Product whose engine-path variable names the Scripted engine.
        script: The script's path in the corpus.
        home: The home to install it in, the test's own unless given.

    Raises:
        ToolsError: The manifest lists no Scripted engine.
    """
    engine = test_env.tools.path(Tool.SCRIPTED_ENGINE)
    home = test_env.home if home is None else home
    (home / SCRIPTED_ENGINE_FOLDER).mkdir(parents=True, exist_ok=True)
    shutil.copyfile(CORPUS / script, home / SCRIPTED_ENGINE_FOLDER / "script.json")
    return {
        **test_env.env,
        "GOODFETCH_HOME": str(home),
        product.engine_path_variable: str(engine),
    }


def scripted_engine_record(home: Path) -> Path:
    """The record `goodfetch-scripted-engine` writes under `home`.

    One flushed JSON line for each message it reads or writes, as it reads
    or writes it, beside the script `scripted_engine_env` installed there.
    """
    return home / SCRIPTED_ENGINE_FOLDER / "record.jsonl"


SCRIPTED_ENGINE_READ_DEADLINE_SECONDS = 10.0
"""How long a test waits for the Scripted engine to read a Client's call."""


def scripted_engine_client_methods(home: Path) -> list[object]:
    """The methods of the Client's messages the Scripted engine under `home` read.

    In their order; empty before the engine writes its record.
    """
    record = scripted_engine_record(home)
    if not record.exists():
        return []
    entries = [json.loads(line) for line in record.read_text("utf-8").splitlines()]
    return [
        entry["message"].get("method") for entry in entries if entry["from"] == "client"
    ]


async def until_scripted_engine_read(home: Path, method: str, count: int = 1) -> None:
    """Waits until the Scripted engine under `home` has read `count` calls of `method`.

    The engine records each message as it reads it, one flushed JSON line
    each, and signals nothing, so the wait polls its record. It fails after
    `SCRIPTED_ENGINE_READ_DEADLINE_SECONDS`.
    """
    with anyio.fail_after(SCRIPTED_ENGINE_READ_DEADLINE_SECONDS):
        while True:
            if scripted_engine_client_methods(home).count(method) >= count:
                return
            await anyio.sleep(0.01)


def _stop(process: subprocess.Popen[bytes]) -> None:
    """Kills `process` if it still runs, and closes its pipes."""
    if process.poll() is None:
        process.kill()
    process.wait()
    for pipe in (process.stdin, process.stdout):
        if pipe is not None:
            pipe.close()
