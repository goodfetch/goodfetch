"""J-044, same SDK program on an Attached engine, then an Exposed Daemon.

"Run the same SDK program on an Attached engine, then an Exposed Daemon by
two variables", on the `sdk-python` surface (ADRs 0049, 0063, 0067, 0078,
0083, 0092 and 0095; CL-06): the same Python SDK program runs with `engine`
left at `auto` first on an Attached engine and then on an Exposed Daemon by
setting `GOODFETCH_URL` and `GOODFETCH_TOKEN`, with no code change. Rust's
`tests/it/journeys/j044.rs` and TypeScript's `tests/journeys/J-044.test.ts`
are its twins.
"""

from __future__ import annotations

import contextlib
import json
import logging
import os
import queue
import re
import subprocess
import threading
from dataclasses import dataclass
from typing import TYPE_CHECKING, TypeVar, cast

import anyio.to_thread
import pytest
from goodfetch_collie import AsyncCollie, EngineInfo, Product
from goodfetch_test_support import (
    Canary,
    JourneyRun,
    NamedOutput,
    journey,
    search_canaries,
)

if TYPE_CHECKING:
    from collections.abc import Mapping
    from pathlib import Path

RUN_ID = "j044-same-program"
"""The Run ID the shared SDK program uses on both engines."""

EXPLAINED_KEY = "timeout.navigation"
"""The setting the shared SDK program explains, unset in both homes."""

STORED_BYTES = b"j044-same-program-bytes"
"""The bytes stored and read back by the shared SDK program."""

LOG_DEADLINE_SECONDS = 60.0
"""How long the test waits for the Exposed Daemon to report its endpoint."""

TOKEN_CANARY = Canary(
    id="j044-exposed-daemon-token",
    kind="endpoint_token",
    value="endpoint-token-canary-j044-000000000044",
)
"""A visibly fake endpoint token canary planted into the Exposed Daemon (ADR 0067)."""

_VERSION_RE = re.compile(r"^goodfetch-collie (\S+)$")

_T = TypeVar("_T")


def present(value: _T | None) -> _T:
    """The value an earlier step gave: a step runs once those before it passed."""
    assert value is not None, "an earlier step gave it"
    return value


def version_line(version: str) -> str:
    """The per-major folder name for `version` (ADR 0030)."""
    parts = version.split(".")
    assert len(parts) >= 2, f"unexpected Product version: {version}"
    return f"0.{parts[1]}" if parts[0] == "0" else parts[0]


def executable_version(engine: Path, env: Mapping[str, str], cwd: Path) -> str:
    """Reads `goodfetch-collie --version`."""
    completed = subprocess.run(
        [str(engine), "--version"],
        env=dict(env),
        cwd=cwd,
        capture_output=True,
        text=True,
        check=True,
    )
    match = _VERSION_RE.fullmatch(completed.stdout.strip())
    assert match is not None, f"unexpected --version output: {completed.stdout!r}"
    return match.group(1)


class _LogCollector(logging.Handler):
    """Captures every log record emitted by `goodfetch.client`."""

    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append(f"{record.getMessage()} {vars(record)}")


@dataclass(frozen=True)
class ProgramResult:
    """The deterministic result of the shared SDK program, compared across engines."""

    run_id: str
    lifecycle: str
    absent: object
    stored: object
    stored_bytes: tuple[bytes, str] | None
    deleted: tuple[bool, bool]
    explained: object


@dataclass(frozen=True)
class ProgramOutcome:
    """The EngineInfo and result from one run of the shared SDK program."""

    engine: EngineInfo
    result: ProgramResult


class ExposedDaemon:
    """An Exposed Daemon started with `daemon run` in a separate Goodfetch home."""

    def __init__(
        self,
        process: subprocess.Popen[str],
        lines: queue.Queue[str | None],
        logged: list[str],
        url: str,
    ) -> None:
        """Holds the running process, its stdout reader queue, and its ws:// URL."""
        self.process = process
        self._lines = lines
        self.logged = logged
        self.url = url

    @classmethod
    def start(
        cls,
        engine: Path,
        base_env: Mapping[str, str],
        folder: Path,
        version: str,
        token: str,
    ) -> ExposedDaemon:
        """Prepares a separate home and starts `goodfetch-collie daemon run`."""
        daemon_home = folder / "daemon-home"
        major_dir = daemon_home / version_line(version)
        major_dir.mkdir(parents=True, exist_ok=True)
        config_path = major_dir / "configuration.toml"
        config_path.write_text(
            'version = "0.1"\n\n[settings]\ndaemon.address = "127.0.0.1:0"\n',
            encoding="utf-8",
        )
        token_path = major_dir / "daemon.token"
        token_path.write_text(token, encoding="utf-8")
        if os.name != "nt":
            config_path.chmod(0o600)
            token_path.chmod(0o600)

        env = {**base_env, "GOODFETCH_HOME": str(daemon_home)}
        process = subprocess.Popen(
            [str(engine), "daemon", "run"],
            env=env,
            cwd=folder,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
        )
        stdout = process.stdout
        assert stdout is not None, "daemon stdout is piped"
        lines: queue.Queue[str | None] = queue.Queue()

        def reader() -> None:
            try:
                for raw in stdout:
                    lines.put(raw.rstrip("\r\n"))
            finally:
                lines.put(None)

        threading.Thread(target=reader, daemon=True).start()
        logged: list[str] = []
        endpoint: str | None = None
        while endpoint is None:
            line = lines.get(timeout=LOG_DEADLINE_SECONDS)
            if line is None:
                message = f"Exposed Daemon exited before serving: {logged}"
                raise RuntimeError(message)
            logged.append(line)
            try:
                parsed: object = json.loads(line)
            except ValueError:
                continue
            if isinstance(parsed, dict):
                fields = cast("dict[str, object]", parsed).get("fields")
                if (
                    isinstance(fields, dict)
                    and cast("dict[str, object]", fields).get("message")
                    == "the foreground Daemon serves"
                ):
                    bound = cast("dict[str, object]", fields).get("endpoint")
                    if isinstance(bound, str):
                        endpoint = bound

        return cls(process, lines, logged, f"ws://{endpoint}")

    def drain_after_stop(self) -> bool:
        """Waits for the Exposed Daemon to exit and drains remaining stdout lines."""
        code = self.process.wait(timeout=LOG_DEADLINE_SECONDS)
        while True:
            try:
                line = self._lines.get(timeout=LOG_DEADLINE_SECONDS)
            except queue.Empty:
                break
            if line is None:
                break
            self.logged.append(line)
        return code == 0

    def kill_if_alive(self) -> None:
        """Ensures no Exposed Daemon process outlives the test."""
        if self.process.poll() is None:
            self.process.kill()
            with contextlib.suppress(subprocess.TimeoutExpired):
                self.process.wait(timeout=10)


async def run_program(env: Mapping[str, str], seed_url: str) -> ProgramOutcome:
    """The shared SDK program: leaves `engine` at `auto`, runs Collie ops."""
    collie = await AsyncCollie.connect(env=env)
    try:
        engine = collie.engine
        started = await collie.start(run_id=RUN_ID)
        store = started.key_value_store
        absent = await store.get("item")
        await store.set("item", {"seed_url": seed_url, "count": 2, "nested": [1, None]})
        stored = await store.get("item")
        await store.set_bytes("note", STORED_BYTES, "text/plain")
        stored_bytes = await store.get_bytes("note")
        deleted = (await store.delete("item"), await store.delete("item"))
        explained = await started.explain_settings([EXPLAINED_KEY])
        report = await started.finish()
        return ProgramOutcome(
            engine=engine,
            result=ProgramResult(
                run_id=report.run_id.root,
                lifecycle=str(report.lifecycle),
                absent=absent,
                stored=stored,
                stored_bytes=(
                    None
                    if stored_bytes is None
                    else (stored_bytes.content, stored_bytes.content_type)
                ),
                deleted=deleted,
                explained=explained.settings,
            ),
        )
    finally:
        await collie.close()


@pytest.mark.covers("story:14-62", "story:14-63", "row:CL-06")
@journey("J-044", "sdk-python")
async def test_j044(run: JourneyRun) -> None:
    """Covers: story:14-62, story:14-63, row:CL-06."""
    proving_ground = run.proving_ground
    assert proving_ground is not None, "J-044 has a Proving ground"
    seed_url = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/2")
    version = await anyio.to_thread.run_sync(
        executable_version, run.engine, run.test_env.env, run.test_env.folder
    )

    daemon: ExposedDaemon | None = None
    local_outcome: ProgramOutcome | None = None
    remote_outcome: ProgramOutcome | None = None
    daemon_stopped = False

    client_logger = logging.getLogger("goodfetch.client")
    collector = _LogCollector()
    previous_level = client_logger.level

    try:
        daemon = await run.step(
            "Start an Exposed Daemon in a separate Goodfetch home on loopback "
            "port 0 with a planted Secret canary as its persistent token",
            lambda: anyio.to_thread.run_sync(
                ExposedDaemon.start,
                run.engine,
                run.test_env.env,
                run.test_env.folder,
                version,
                TOKEN_CANARY.value,
            ),
        )
        local_outcome = await run.step(
            "Run the SDK program with `engine` left at `auto` and neither "
            "`GOODFETCH_URL` nor `GOODFETCH_TOKEN` set, reading seeded Proving "
            "ground input, storing and reading values in the Run's Key-value "
            "store, explaining settings, and finishing the Run",
            lambda: run_program(run.env, seed_url),
        )

        async def run_on_exposed() -> ProgramOutcome:
            exposed = present(daemon)
            remote_env = {
                **run.env,
                "GOODFETCH_URL": exposed.url,
                "GOODFETCH_TOKEN": TOKEN_CANARY.value,
            }
            client_logger.addHandler(collector)
            client_logger.setLevel(logging.DEBUG)
            try:
                return await run_program(remote_env, seed_url)
            finally:
                client_logger.removeHandler(collector)
                client_logger.setLevel(previous_level)

        remote_outcome = await run.step(
            "Run the exact same SDK program with `GOODFETCH_URL` pointing at "
            "the Exposed Daemon and `GOODFETCH_TOKEN` carrying the Secret "
            "canary, with no code change",
            run_on_exposed,
        )

        async def stop_exposed() -> bool:
            exposed = present(daemon)
            remote_env = {
                **run.env,
                "GOODFETCH_URL": exposed.url,
                "GOODFETCH_TOKEN": TOKEN_CANARY.value,
            }
            stop_conn = await AsyncCollie.connect(env=remote_env)
            try:
                await stop_conn.call("daemon.stop", {})
            finally:
                with contextlib.suppress(Exception):
                    await stop_conn.close()
            return await anyio.to_thread.run_sync(exposed.drain_after_stop)

        stopped = await run.step(
            "Stop the Exposed Daemon cleanly and collect its logs",
            stop_exposed,
        )
        daemon_stopped = bool(stopped)

        async def check_identical() -> None:
            local = present(local_outcome)
            remote = present(remote_outcome)
            assert local.engine.kind == "attached"
            assert remote.result == local.result
            assert daemon_stopped is True

        async def check_engine_info() -> None:
            exposed = present(daemon)
            remote = present(remote_outcome)
            assert remote.engine == EngineInfo(
                kind="remote",
                product=Product.collie,
                version=version,
                protocol_version="0.1",
                project_file=None,
                home=None,
                engine_path=None,
                url=exposed.url,
                fallback=None,
            )

        async def check_no_canary() -> None:
            exposed = present(daemon)
            remote = present(remote_outcome)
            hits = search_canaries(
                [TOKEN_CANARY],
                [
                    NamedOutput("engine_repr", repr(remote.engine)),
                    NamedOutput("engine_str", str(remote.engine)),
                    NamedOutput("sdk_logs", "\n".join(collector.lines)),
                    NamedOutput("daemon_logs", "\n".join(exposed.logged)),
                    NamedOutput("program_output", repr(remote.result)),
                ],
            )
            assert hits == []

        await run.check(
            "The program returned identical results on the local engine and "
            "on the Exposed Daemon",
            check_identical,
        )
        await run.check(
            "On the Exposed Daemon, `EngineInfo` reported `kind` as `remote`, "
            "the `url` reached, Product `collie`, the executable's Product "
            "version, and protocol version `0.1`, with no home, engine path, "
            "or fallback",
            check_engine_info,
        )
        await run.check(
            "The Secret canary in `GOODFETCH_TOKEN` appeared nowhere in "
            "`EngineInfo`, SDK logs, Exposed Daemon logs, or program outputs",
            check_no_canary,
        )
    finally:
        if daemon is not None:
            daemon.kill_if_alive()
