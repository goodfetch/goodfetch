"""Collie's testing kit (issue 3398, ADR 0105), on the real engine.

`AsyncTestEngine` and `TestEngine` start an Attached engine of Collie's
executable of the same commit, three ways: async on asyncio, async on Trio
and through the sync `TestEngine`. It runs in a temporary Goodfetch home it
removes on close, with a cleared environment and `egress.offline` on; a host
`network.resolve` maps to loopback alone is reached live and any other is
refused, and browsers come from the caller's Browser cache, which the engine
reads and never writes: the kit turns `browser_cache.read_only` on (issue
3458). With a Daemon running in the caller's home, the code under test runs
unchanged on the Daemon's connection and on the test engine's, which reaches
its own Attached engine. The kit's sample tests run under pytest with AnyIO's
plugin alone and under `unittest` (issue 3399). Rust's
`tests/it/sdk/test_engine.rs` is its twin.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import anyio.to_thread
import pytest
from collie_calls import failure, open_collie, response_on
from goodfetch_collie import (
    ArtifactUnavailableOfflineError,
    AsyncCollie,
    Collie,
    EgressRefusedError,
    Settings,
    testing,
)
from goodfetch_test_support import (
    BrowserDownload,
    ClientCaseRecord,
    ProvingGround,
    TestEnv,
    Way,
    three_ways,
)

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client import testing as client_testing

CASE = "test-engine-runs-the-code-under-test"
"""The Testing-kit case these tests play."""

PROGRAMS = Path(__file__).parents[1] / "programs"
"""The folder of the programs the tests run, the kit's sample tests among them."""

SAMPLE_DEADLINE_SECONDS = 120.0
"""How long a runner may take over the kit's sample tests, their engines included."""

PUBLIC_HOST = "public.test"
"""A host the test's `network.resolve` maps to an address that is not loopback.

TEST-NET-1's, which no Run reaches.
"""

Entry = tuple[bool, int, int]
"""An entry of a Browser cache: whether it is a folder, its length and mtime."""


async def start(
    way: Way,
    env: Mapping[str, str],
    resolve: Mapping[str, list[str]] | None = None,
) -> client_testing.AsyncTestEngine[AsyncCollie]:
    """A test engine this way started with `env` and `network.resolve`'s `resolve`."""
    settings: Settings = {} if resolve is None else {"network": {"resolve": resolve}}
    return await way.open(
        lambda: testing.AsyncTestEngine.start(env=env, settings=settings),
        lambda: testing.TestEngine.start(env=env, settings=settings),
    )


def connection_of(
    way: Way, engine: client_testing.AsyncTestEngine[AsyncCollie]
) -> AsyncCollie:
    """`engine`'s connection, its methods async whatever the way.

    It is a property, so the sync way's proxy hands the sync `Collie` itself,
    which `way.api` wraps.
    """
    connection = engine.connection
    return way.api(connection, connection)


def tree(cache: Path) -> dict[str, Entry]:
    """Every entry of the Browser cache at `cache`, by its path under `cache`.

    Its kind, length and modification time, symbolic links unfollowed: the
    cache's own records, and not a build's `files` folder, where no engine
    writes and the browser it launches keeps its own folders.
    """
    entries: dict[str, Entry] = {}
    for folder, folders, files in os.walk(cache):
        here = Path(folder)
        folders[:] = [
            name
            for name in folders
            if not (name == "files" and len(here.relative_to(cache).parts) == 3)
        ]
        for name in [*folders, *files]:
            path = here / name
            status = path.lstat()
            entries[path.relative_to(cache).as_posix()] = (
                path.is_dir() and not path.is_symlink(),
                status.st_size,
                status.st_mtime_ns,
            )
    return entries


async def finished(collie: AsyncCollie) -> str:
    """The code under test: it starts a Run on `collie` and answers how it ended."""
    return (await (await collie.start()).finish()).lifecycle


@pytest.mark.covers("story:11-156", "story:11-157", "story:11-168", "story:11-172")
@pytest.mark.client_case(id=CASE, mode="attached")
@three_ways
async def test_a_test_runs_its_code_on_the_connection_and_close_removes_its_home(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """The Run finishes on `engine.connection`; its home is gone after `close()`."""
    engine = await start(way, env)
    own_type = type(engine.connection)
    connection = connection_of(way, engine)
    home = connection.engine.home
    assert home is not None, "an Attached engine names its home"
    existed = await anyio.to_thread.run_sync(os.path.isdir, home)

    lifecycle = await finished(connection)
    await engine.close()

    with client_case.check("engine.connection is the Product's own connection type"):
        assert own_type is (Collie if way is Way.SYNC else AsyncCollie)
    with client_case.check("the code under test finishes a Run on engine.connection"):
        assert lifecycle == "finished"
    with client_case.check(
        "the engine ran in a temporary Goodfetch home of its own, removed on close"
    ):
        assert Path(home) != Path(env["GOODFETCH_HOME"])
        assert (existed, await anyio.to_thread.run_sync(os.path.exists, home)) == (
            True,
            False,
        ), home


@pytest.mark.covers("story:11-157", "story:11-169")
@pytest.mark.client_case(id=CASE, mode="daemon")
@pytest.mark.parametrize("with_daemon", [True])
@three_ways
async def test_the_code_under_test_runs_unchanged_on_a_daemon_and_on_the_test_engine(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """The Daemon in the caller's home answers `connect()`, never the test engine."""
    collie = await open_collie(way, env, engine="daemon")
    on_the_daemon = await finished(collie)
    daemon = collie.engine
    await collie.close()
    engine = await start(way, env)
    connection = connection_of(way, engine)
    on_the_test_engine = await finished(connection)
    reached = connection.engine
    await engine.close()

    with client_case.check(
        "the code under test finishes a Run on a Daemon's connection and on the "
        "test engine's"
    ):
        assert (on_the_daemon, on_the_test_engine) == ("finished", "finished")
    with client_case.check(
        "the test engine reaches its own Attached engine in its own home, never "
        "the Daemon of the caller's"
    ):
        assert (daemon.kind, reached.kind) == ("daemon", "attached")
        assert reached.home is not None
        assert Path(reached.home) != Path(env["GOODFETCH_HOME"])


def sample_folder(test_env: TestEnv, *samples: str) -> Path:
    """A program's own folder holding the kit's `samples`, none of the workspace's.

    An empty `pytest.ini` makes it pytest's root, so no configuration of
    the workspace's applies.
    """
    folder = test_env.folder / "program"
    folder.mkdir()
    (folder / "pytest.ini").write_text("[pytest]\n", "utf-8")
    for sample in samples:
        shutil.copy(PROGRAMS / sample, folder / sample)
    return folder


def run_sample(
    command: list[str], folder: Path, env: Mapping[str, str]
) -> subprocess.CompletedProcess[str]:
    """`command` run in `folder` with the test's environment and no plugin autoloaded.

    The engine comes from the environment, as a test run sets it, and
    nothing writes bytecode into the folder.
    """
    return subprocess.run(
        [sys.executable, *command],
        capture_output=True,
        text=True,
        cwd=folder,
        env={
            **env,
            "PYTEST_DISABLE_PLUGIN_AUTOLOAD": "1",
            "PYTHONDONTWRITEBYTECODE": "1",
        },
        timeout=SAMPLE_DEADLINE_SECONDS,
        check=False,
    )


@pytest.mark.covers("story:11-171", "story:11-172")
def test_the_kits_sample_test_passes_under_pytest_with_anyio_alone(
    env: Mapping[str, str], test_env: TestEnv
) -> None:
    """On asyncio, on Trio and through the sync API, `TestEngine` never collected.

    Collection warnings are errors, so a `TestEngine` pytest tried to
    collect fails the run.
    """
    folder = sample_folder(test_env, "kit_sample_pytest.py")

    ran = run_sample(
        [
            "-m",
            "pytest",
            "-p",
            "anyio.pytest_plugin",
            "-p",
            "no:cacheprovider",
            "-W",
            "error::pytest.PytestCollectionWarning",
            "-rA",
            "kit_sample_pytest.py",
        ],
        folder,
        env,
    )

    summary = ran.stdout.splitlines()[-1]
    outcomes = re.findall(r"\d+ (?:passed|failed|errors?|skipped|x\w+)", summary)
    assert (ran.returncode, outcomes) == (0, ["3 passed"]), ran.stdout + ran.stderr


@pytest.mark.covers("story:11-171", "story:11-172")
def test_the_kits_sample_test_passes_under_unittest(
    env: Mapping[str, str], test_env: TestEnv
) -> None:
    """The async test on `IsolatedAsyncioTestCase`; the same test on the sync API."""
    folder = sample_folder(test_env, "kit_sample_unittest.py")

    ran = run_sample(["-m", "unittest", "kit_sample_unittest"], folder, env)

    assert (
        ran.returncode,
        re.findall(r"^Ran (\d+) tests? in", ran.stderr, re.MULTILINE),
        ran.stderr.splitlines()[-1],
    ) == (0, ["2"], "OK"), ran.stderr


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests."""
    return 3398


@pytest.mark.covers("story:11-168", "story:11-169")
@pytest.mark.parametrize("with_browser", [True])
@three_ways
async def test_a_loopback_resolve_entry_is_reached_any_other_host_is_refused(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """Live through a loopback `resolve` entry; refused elsewhere; the cache unchanged.

    The caller's Browser cache stays writable here: the kit's
    `browser_cache.read_only` alone keeps the engine from recording the
    build's use in it (ADR 0098).
    """
    cache = Path(env["GOODFETCH_HOME"]) / "browsers"
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    resolve[PUBLIC_HOST] = ["192.0.2.7"]
    reached = await anyio.to_thread.run_sync(proving_ground.start_url, "statuses/1")
    before = tree(cache)
    engine = await start(way, env, resolve)
    run = await connection_of(way, engine).start()
    page = await run.new_page()

    navigated = await page.goto(reached)
    assert navigated is not None, "a document has a response"
    status = response_on(way, navigated).status
    refused = await failure(page.goto(proving_ground.url(PUBLIC_HOST, "/")))
    await run.finish()
    await engine.close()

    assert (status, type(refused)) == (200, EgressRefusedError)
    assert tree(cache) == before


@pytest.mark.covers("story:11-169")
@pytest.mark.parametrize("browser_download", [BrowserDownload.NOT_FOUND])
@three_ways
async def test_with_no_browser_installed_the_first_page_fails_offline(
    way: Way, env: Mapping[str, str]
) -> None:
    """`artifact_unavailable_offline`, its `fix` naming `browser install`.

    The caller's empty Browser cache stays empty.
    """
    cache = Path(env["GOODFETCH_BROWSERS_PATH"])
    engine = await start(way, env)
    run = await connection_of(way, engine).start()
    refused = await failure(run.new_page())
    await run.finish()
    await engine.close()

    assert isinstance(refused, ArtifactUnavailableOfflineError)
    assert refused.fix == (
        "Run `goodfetch-collie browser install` beforehand where the Browser"
        " cache is writable, or lift `egress.offline` where it was set"
    )
    assert tree(cache) == {}
