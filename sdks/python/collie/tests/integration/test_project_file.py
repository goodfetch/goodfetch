"""The project file `connect()` reads (ADR 0083; story 11-75).

On the Scripted engine (ADR 0069): `project` names it, `engine` reports it,
`config.explain` and `run.start` carry it to the engine, whose script
expects exactly those params, and its `timeout.connect` bounds `hello`
unless the connection sets its own. A remote Daemon refuses a value the
project file has no authority for there with `setting_locked`.
`read_project`'s own tests cover how the file is found and refused.
TypeScript's `project-file.test.ts` is its twin.
"""

from datetime import timedelta
from pathlib import Path

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


def write_project(folder: Path, text: str) -> Path:
    """Writes `text` as the project file `goodfetch.toml` in `folder`, creating it."""
    folder.mkdir(parents=True, exist_ok=True)
    file = folder / "goodfetch.toml"
    file.write_text(text, "utf-8")
    return file


@pytest.mark.covers("story:11-75", "operation:config.explain", "operation:run.start")
@three_ways
async def test_reaches_the_engine_on_config_explain_and_run_start_and_engine_reports_it(
    way: Way, test_env: TestEnv, product: Product
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

    assert (collie.engine.project_file, report.lifecycle) == (str(project), "finished")


@pytest.mark.covers("story:11-75", "setting:timeout.connect", "error:deadline_exceeded")
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
@three_ways
async def test_is_refused_by_a_remote_daemon_when_a_value_widens_tls_trusted_cas(
    way: Way, test_env: TestEnv, product: Product
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

    assert await exit_code(daemon.wait) == 0
    assert isinstance(refused, SettingLockedError)
    assert (refused.details["key"], refused.details["layer"]) == (
        "tls.trusted_cas",
        "project_configuration",
    )
