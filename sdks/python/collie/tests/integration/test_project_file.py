"""The project file `connect()` reads (ADR 0083; story 11-75).

The `connect-reads-the-project-file` Client conformance case, on the
Scripted engine (ADR 0069). By default the file is found upward from the
working directory, shown in a program the test starts below it, so no test
moves its own; `project` names another and `None` reads none. `engine`
reports it, and `config.explain` and `run.start` carry it to the engine,
whose script expects exactly those params. Its `timeout.connect` bounds
`hello` unless the connection sets its own. A remote Daemon refuses a value
the project file has no authority for there with `setting_locked`.
`read_project`'s own tests cover how the file is refused. TypeScript's
`project-file.test.ts` is its twin.
"""

import json
import sys
from datetime import timedelta
from pathlib import Path

import anyio
import pytest
from collie_calls import exit_code, failure
from goodfetch_collie import (
    AsyncCollie,
    Collie,
    DeadlineExceededError,
    RemoteDaemon,
    SettingLockedError,
)
from goodfetch_collie_client.generated.connection import WEBSOCKET_SUBPROTOCOL
from goodfetch_collie_client.handle import NotGiven, not_given
from goodfetch_test_support import (
    ClientCaseRecord,
    Product,
    TestEnv,
    Way,
    project_script,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)

TOKEN = "endpoint-token-canary-5f0e9b27c4d1"
"""The remote Daemon's token in its script.

`sdk-python/connect/project-value-locked-by-a-remote-daemon.json`.
"""

PROGRAM = Path(__file__).parents[1] / "programs" / "collie_project.py"
"""The program that connects from the working directory the test gives it."""

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long the program may take to connect, play its Run and close."""


def write_project(folder: Path, text: str) -> Path:
    """Writes `text` as the project file `goodfetch.toml` in `folder`, creating it."""
    folder.mkdir(parents=True, exist_ok=True)
    file = folder / "goodfetch.toml"
    file.write_text(text, "utf-8")
    return file


@pytest.mark.covers("story:11-75", "operation:config.explain", "operation:run.start")
@pytest.mark.client_case(id="connect-reads-the-project-file", mode="attached")
@three_ways
async def test_reaches_the_engine_on_config_explain_and_run_start_and_engine_reports_it(
    way: Way, test_env: TestEnv, product: Product, client_case: ClientCaseRecord
) -> None:
    """The script expects each call's params.

    So it plays to its end only when both carry the file read.
    """
    project = write_project(
        test_env.folder / "work",
        'version = "0.1"\n\n[settings.timeout]\nnavigation = 45000\n',
    )
    env = scripted_engine_env(
        test_env,
        product,
        project_script(test_env, "sdk-python/run/project-file-carried.json", project),
    )

    collie = await way.open(
        lambda: AsyncCollie.connect(env=env, project=project),
        lambda: Collie.connect(env=env, project=project),
    )
    await collie.explain_settings(["timeout.navigation"])
    run = await collie.start()
    report = await run.finish()
    await collie.close()

    with client_case.check(
        "the file project names reached the engine on config.explain and run.start,"
        " and the engine info reports it"
    ):
        assert (collie.engine.project_file, report.lifecycle) == (
            str(project),
            "finished",
        )


@pytest.mark.covers("story:11-75", "operation:config.explain", "operation:run.start")
@pytest.mark.client_case(id="connect-reads-the-project-file", mode="attached")
@three_ways
async def test_is_found_upward_from_the_working_directory_and_none_reads_none(
    way: Way, test_env: TestEnv, product: Product, client_case: ClientCaseRecord
) -> None:
    """As the program started two folders below the project file sees it."""
    project = write_project(
        test_env.folder / "project",
        'version = "0.1"\n\n[settings.timeout]\nnavigation = 45000\n',
    ).resolve()
    deeper = test_env.folder / "project" / "src" / "deeper"
    deeper.mkdir(parents=True)
    scripts = {
        "found": project_script(
            test_env, "sdk-python/run/project-file-carried.json", project
        ),
        "none": "sdk-python/run/project-file-read-none.json",
    }

    seen: dict[str, tuple[int, str, object]] = {}
    for chosen, script in scripts.items():
        out = test_env.folder / f"{chosen}.json"
        env = scripted_engine_env(test_env, product, script, test_env.folder / chosen)
        with anyio.fail_after(PROGRAM_DEADLINE_SECONDS):
            program = await anyio.run_process(
                [sys.executable, str(PROGRAM), str(out), way.value, chosen],
                cwd=deeper,
                env=env,
                check=False,
            )
        played = json.loads(out.read_text("utf-8")) if out.exists() else None
        seen[chosen] = (
            program.returncode,
            program.stderr.decode(errors="replace"),
            played,
        )

    with client_case.check(
        "with no project, the file found upward from the working directory"
        " reached the engine, and the engine info reports it"
    ):
        assert seen["found"] == (
            0,
            seen["found"][1],
            {"project_file": str(project), "lifecycle": "finished"},
        )
    with client_case.check("with project None, none reached the engine"):
        assert seen["none"] == (
            0,
            seen["none"][1],
            {"project_file": None, "lifecycle": "finished"},
        )


@pytest.mark.covers("story:11-75", "setting:timeout.connect", "error:deadline_exceeded")
@pytest.mark.client_case(id="connect-reads-the-project-file", mode="attached")
@pytest.mark.parametrize(
    ("in_file", "connect_timeout", "limit"),
    [
        pytest.param(
            200,
            not_given,
            ("timeout.connect", 200, "project_configuration", None),
            id="its-timeout-connect-naming-the-project-files-layer",
        ),
        pytest.param(
            60_000,
            timedelta(milliseconds=200),
            ("timeout.connect", 200, "run", "argument"),
            id="the-connections-own-timeout-over-its-timeout-connect",
        ),
    ],
)
@three_ways
async def test_bounds_hello(
    way: Way,
    test_env: TestEnv,
    product: Product,
    client_case: ClientCaseRecord,
    in_file: int,
    connect_timeout: timedelta | NotGiven,
    limit: tuple[str, int, str, str | None],
) -> None:
    """The file's `timeout.connect` bounds `hello`, unless `timeout` is given."""
    project = write_project(
        test_env.folder / "work",
        f'version = "0.1"\n\n[settings.timeout]\nconnect = {in_file}\n',
    )
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/hello-never-answered.json"
    )
    error = await failure(
        way.open(
            lambda: AsyncCollie.connect(
                env=env, project=project, timeout=connect_timeout
            ),
            lambda: Collie.connect(env=env, project=project, timeout=connect_timeout),
        )
    )

    with client_case.check(
        "hello unanswered failed with deadline_exceeded, its limit naming"
        " the timeout.connect that bounded it"
    ):
        assert isinstance(error, DeadlineExceededError)
        assert error.limit is not None
        assert (
            error.limit.key,
            error.limit.value,
            error.limit.layer,
            error.limit.origin,
        ) == limit
        assert error.details["waited_for"] == "hello"


@pytest.mark.covers("story:11-75", "error:setting_locked", "operation:run.start")
@pytest.mark.client_case(id="connect-reads-the-project-file", mode="daemon")
@three_ways
async def test_is_refused_by_a_remote_daemon_when_a_value_widens_tls_trusted_cas(
    way: Way, test_env: TestEnv, product: Product, client_case: ClientCaseRecord
) -> None:
    """With `setting_locked` at `project_configuration` (ADR 0083)."""
    project = write_project(
        test_env.folder / "work",
        'version = "0.1"\n\n[settings.tls.trusted_cas]\n'
        '"intranet.example" = ["/etc/ssl/intranet-ca.pem"]\n',
    )
    daemon = start_scripted_daemon(
        test_env,
        project_script(
            test_env,
            "sdk-python/connect/project-value-locked-by-a-remote-daemon.json",
            project,
        ),
        "websocket",
        subprotocol=WEBSOCKET_SUBPROTOCOL,
    )
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/reached-attached.json"
    )
    engine = RemoteDaemon(f"ws://{daemon.address}", TOKEN)

    collie = await way.open(
        lambda: AsyncCollie.connect(env=env, project=project, engine=engine),
        lambda: Collie.connect(env=env, project=project, engine=engine),
    )
    refused = await failure(collie.start())
    await collie.close()

    with client_case.check(
        "the remote Daemon read the project file on run.start and played its script"
    ):
        assert await exit_code(daemon.wait) == 0
    with client_case.check(
        "the Run's start raised setting_locked on tls.trusted_cas"
        " at project_configuration"
    ):
        assert isinstance(refused, SettingLockedError)
        assert (refused.details["key"], refused.details["layer"]) == (
            "tls.trusted_cas",
            "project_configuration",
        )
