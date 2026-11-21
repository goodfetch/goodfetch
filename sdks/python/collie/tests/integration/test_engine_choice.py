"""Which engine `connect()` reaches (ADRs 0024, 0078 and 0083).

With `engine` left at `auto`, the remote Daemon `GOODFETCH_URL` names, else
the Daemon whose runtime file is in the Goodfetch home, else an Attached
engine; `attached`, `daemon` and `RemoteDaemon(url, token)` choose one,
beating `GOODFETCH_URL`. Each plays a Scripted engine's script (ADR 0069),
as an Attached engine, as a Daemon on a Unix socket, or on loopback TCP. A
Scripted Daemon exits 0 only once every step played, so its exit says the
Client sent exactly what the script holds, the Daemon's token in `hello`
included. `test_engine_choice_daemon.py` reaches the Daemon of the same
commit. TypeScript's `engine-choice.test.ts` is its twin.
"""

import json
import logging
from collections.abc import Callable, Mapping
from pathlib import Path

import anyio.to_thread
import pytest
from collie_calls import CLIENT_LOGGER, engine_warnings, failure, open_collie
from goodfetch_collie import (
    DaemonUnreachableError,
    EngineInfo,
    EngineNotFoundError,
    GoodfetchError,
    Product,
    RemoteDaemon,
    SettingInvalidError,
)
from goodfetch_test_support import (
    Canary,
    CanaryHit,
    ClientCaseRecord,
    NamedOutput,
    TestEnv,
    Way,
    scripted_engine_env,
    search_canaries,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

CANARY = Canary(
    id="remote-daemon-token",
    kind="endpoint_token",
    value="endpoint-token-canary-5f0e9b27c4d1",
)
"""The token `connect/reached-remote-with-a-canary-token.json` takes (ADR 0067)."""

ATTACHED = "sdk-python/connect/reached-attached.json"
"""The script of an Attached engine, which a test reaching a Daemon never starts."""

DAEMON = "sdk-python/connect/reached-daemon.json"
"""The script a Daemon of the home plays: its `hello` carries the Daemon's token."""

REMOTE = "sdk-python/connect/reached-remote-with-a-canary-token.json"
"""The script a remote Daemon plays: its `hello` carries the canary token."""


def write_runtime_file(home: Path, contents: str) -> None:
    """Writes a runtime file in `home`'s per-major folder, as a Daemon publishes one.

    `0.0` is the folder of the Scripted engine's version 0.0.0 (ADR 0030).
    """
    folder = home / "0.0"
    folder.mkdir(parents=True, exist_ok=True)
    file = folder / "daemon.json"
    file.write_text(contents, "utf-8")
    file.chmod(0o600)


def reached(caplog: pytest.LogCaptureFixture) -> list[tuple[int, str, object]]:
    """Each line `goodfetch.client` got: its level, its message and its `engine`."""
    return [
        (record.levelno, record.getMessage(), record.__dict__.get("engine"))
        for record in caplog.records
        if record.name == CLIENT_LOGGER
    ]


def shown_text(error: GoodfetchError) -> str:
    """All `error` shows: its message, `fix`, details and `repr`."""
    return f"{error} {error.fix} {error.details} {error!r} {vars(error)}"


def shown(error: GoodfetchError) -> NamedOutput:
    """All `error` shows, named for canary hits."""
    return NamedOutput("error", shown_text(error))


def logs(caplog: pytest.LogCaptureFixture) -> NamedOutput:
    """Every line logged, with its fields."""
    return NamedOutput(
        "logs", "\n".join(f"{vars(record)}" for record in caplog.records)
    )


def search_canaries_in(outputs: list[NamedOutput]) -> list[CanaryHit]:
    """Each place `outputs` hold the canary token."""
    return search_canaries([CANARY], outputs)


async def exit_code(wait: Callable[[], int]) -> int:
    """The exit code `wait`, a process's blocking wait, returns, off the event loop."""
    return await anyio.to_thread.run_sync(wait)


@pytest.mark.covers("story:11-2", "story:11-4", "story:11-5", "story:11-8")
@pytest.mark.client_case(id="connect-finds-the-engine", mode="attached")
@three_ways
async def test_reaches_an_attached_engine_when_no_daemon_runs_and_says_so(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """With `engine` at `auto`, no `GOODFETCH_URL` and no Daemon, an Attached engine."""
    caplog.set_level(logging.INFO, logger=CLIENT_LOGGER)
    env = scripted_engine_env(test_env, product, ATTACHED)

    collie = await open_collie(way, env)
    engine = collie.engine
    await collie.close()

    with client_case.check(
        "engine reports an Attached engine, its executable and the home it runs in"
    ):
        assert engine == EngineInfo(
            kind="attached",
            product=Product.collie,
            version="0.0.0",
            protocol_version="0.1",
            project_file=None,
            home=str(test_env.home),
            engine_path=env["GOODFETCH_COLLIE_ENGINE_PATH"],
            url=None,
            fallback=None,
        )
    with client_case.check("the choice is logged once at info, and nothing else"):
        [(level, message, logged)] = reached(caplog)
        assert (level, logged) == (logging.INFO, engine)
        assert "Attached engine" in message


@pytest.mark.covers("story:11-2", "story:11-4", "story:11-5", "story:11-8")
@pytest.mark.client_case(id="connect-finds-the-engine", mode="daemon")
@three_ways
async def test_reaches_the_daemon_whose_runtime_file_is_in_the_home_and_says_so(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The Daemon on a Unix socket, its token in `hello`."""
    caplog.set_level(logging.INFO, logger=CLIENT_LOGGER)
    env = scripted_engine_env(test_env, product, ATTACHED)
    daemon = start_scripted_daemon(test_env, DAEMON, "socket")

    collie = await open_collie(way, env)
    engine = collie.engine
    await collie.close()
    code = await exit_code(daemon.wait)

    with client_case.check(
        "the Daemon played its script through: hello carried its token, and the"
        " close ended the Connection"
    ):
        assert code == 0
    with client_case.check("engine reports the Daemon, where it listens and its home"):
        assert engine == EngineInfo(
            kind="daemon",
            product=Product.collie,
            version="0.0.0",
            protocol_version="0.1",
            project_file=None,
            home=str(test_env.home),
            engine_path=None,
            url=daemon.address,
            fallback=None,
        )
    with client_case.check("the choice is logged once at info"):
        [(level, message, logged)] = reached(caplog)
        assert (level, logged) == (logging.INFO, engine)
        assert "Daemon" in message


@pytest.mark.covers("story:11-4")
@pytest.mark.client_case(id="connect-finds-the-engine", mode="daemon")
@three_ways
async def test_reaches_the_daemon_whose_runtime_file_names_a_loopback_tcp_address(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
) -> None:
    """A runtime file naming loopback TCP, as on Windows, is reached the same way."""
    daemon = start_scripted_daemon(test_env, DAEMON, "tcp")
    write_runtime_file(
        test_env.home,
        json.dumps(
            {
                "endpoint": {"tcp": daemon.address},
                "token": "scripted-daemon-token",
                "version": "0.0.0",
                "pid": 0,
            }
        ),
    )
    env = scripted_engine_env(test_env, product, ATTACHED)

    collie = await open_collie(way, env)
    engine = collie.engine
    await collie.close()
    code = await exit_code(daemon.wait)

    with client_case.check(
        "the Daemon on loopback TCP played its script through, its token in hello"
    ):
        assert code == 0
    with client_case.check("engine reports the Daemon's address and home"):
        assert (engine.kind, engine.home, engine.url) == (
            "daemon",
            str(test_env.home),
            daemon.address,
        )


@pytest.mark.covers("story:11-4", "story:11-5")
@three_ways
async def test_reaches_the_remote_daemon_of_goodfetch_url_and_shows_its_token_nowhere(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """`GOODFETCH_URL` on loopback TCP, with `GOODFETCH_TOKEN`'s canary token."""
    caplog.set_level(logging.DEBUG, logger=CLIENT_LOGGER)
    daemon = start_scripted_daemon(test_env, REMOTE, "tcp")
    url = f"ws://{daemon.address}"
    env = {
        **scripted_engine_env(test_env, product, ATTACHED),
        "GOODFETCH_URL": url,
        "GOODFETCH_TOKEN": CANARY.value,
    }

    collie = await open_collie(way, env)
    engine = collie.engine
    await collie.close()

    assert await exit_code(daemon.wait) == 0
    assert engine == EngineInfo(
        kind="remote",
        product=Product.collie,
        version="0.0.0",
        protocol_version="0.1",
        project_file=None,
        home=None,
        engine_path=None,
        url=url,
        fallback=None,
    )
    assert (
        search_canaries_in(
            [NamedOutput("engine", f"{engine!r} {vars(engine)}"), logs(caplog)]
        )
        == []
    )


@pytest.mark.covers("story:11-5")
@three_ways
async def test_takes_a_remote_daemon_written_in_code_over_goodfetch_url(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """`RemoteDaemon(url, token)` beats `GOODFETCH_URL`, which is never reached."""
    daemon = start_scripted_daemon(test_env, REMOTE, "tcp")
    env = {
        **scripted_engine_env(test_env, product, ATTACHED),
        # Nothing listens here: reading it would fail the connect.
        "GOODFETCH_URL": "ws://127.0.0.1:1",
        "GOODFETCH_TOKEN": "not-the-token",
    }
    url = f"ws://localhost:{daemon.address.rsplit(':', 1)[1]}"

    collie = await open_collie(way, env, engine=RemoteDaemon(url, CANARY.value))
    engine = collie.engine
    await collie.close()

    assert await exit_code(daemon.wait) == 0
    assert (engine.kind, engine.url) == ("remote", url)
    assert CANARY.value not in repr(RemoteDaemon(url, CANARY.value))


@pytest.mark.covers("story:11-5")
@three_ways
async def test_takes_engine_attached_over_goodfetch_url_and_a_running_daemon(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """`attached` starts an Attached engine whatever the environment says."""
    env = {
        **scripted_engine_env(test_env, product, ATTACHED),
        "GOODFETCH_URL": "ws://127.0.0.1:1",
        "GOODFETCH_TOKEN": CANARY.value,
    }
    # A running Daemon, which the Client must pass over.
    daemon = start_scripted_daemon(test_env, DAEMON, "socket")

    collie = await open_collie(way, env, engine="attached")
    engine = collie.engine
    await collie.close()
    daemon.kill()

    assert (engine.kind, engine.url) == ("attached", None)


@pytest.mark.covers("story:11-5", "error:daemon_unreachable")
@three_ways
async def test_fails_with_daemon_unreachable_when_nothing_answers_at_the_url(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The fix names the URL; the token shows in no message."""
    caplog.set_level(logging.DEBUG, logger=CLIENT_LOGGER)
    env = scripted_engine_env(test_env, product, ATTACHED)

    error = await failure(
        open_collie(way, env, engine=RemoteDaemon("ws://127.0.0.1:1", CANARY.value))
    )

    assert isinstance(error, DaemonUnreachableError)
    assert error.group == "unavailable"
    assert "ws://127.0.0.1:1" in (error.fix or "")
    assert search_canaries_in([shown(error), logs(caplog)]) == []


@pytest.mark.covers("story:11-5", "error:daemon_unreachable")
@three_ways
async def test_fails_with_daemon_unreachable_naming_goodfetch_token_on_close_at_hello(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """A Daemon that closes the Connection before it answers `hello`."""
    daemon = start_scripted_daemon(test_env, REMOTE, "tcp")
    env = {
        **scripted_engine_env(test_env, product, ATTACHED),
        "GOODFETCH_URL": f"ws://{daemon.address}",
        "GOODFETCH_TOKEN": "a-token-it-does-not-take",
    }

    error = await failure(open_collie(way, env))
    # The script expected another token: the Scripted engine fails its play.
    code = await exit_code(daemon.wait)

    assert code != 0
    assert isinstance(error, DaemonUnreachableError)
    assert "GOODFETCH_TOKEN" in (error.fix or "")
    assert "a-token-it-does-not-take" not in shown_text(error)


@pytest.mark.covers("story:11-4")
@three_ways
async def test_takes_a_runtime_file_that_holds_no_endpoint_for_a_dead_daemons(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A half-written runtime file leaves `auto` to start an Attached engine."""
    write_runtime_file(test_env.home, "{ half-written")
    env = scripted_engine_env(test_env, product, ATTACHED)

    collie = await open_collie(way, env)
    engine = collie.engine
    await collie.close()

    assert (engine.kind, engine.url) == ("attached", None)
    assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-2", "error:engine_not_found")
@three_ways
async def test_fails_with_engine_not_found_when_engine_daemon_finds_no_executable(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """`daemon` with no Daemon running and no executable to start one."""
    missing = str(test_env.home / "no-collie-here")
    env = {
        **scripted_engine_env(test_env, product, ATTACHED),
        "GOODFETCH_COLLIE_ENGINE_PATH": missing,
    }

    error = await failure(open_collie(way, env, engine="daemon"))

    assert isinstance(error, EngineNotFoundError)
    assert error.details == {"path": missing}
    assert "GOODFETCH_COLLIE_ENGINE_PATH" in (error.fix or "")


@pytest.mark.covers("story:11-5", "error:setting_invalid")
@three_ways
async def test_refuses_a_goodfetch_url_holding_a_password_or_a_query(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Such a URL shows in no message, since what it holds may be a secret."""
    caplog.set_level(logging.DEBUG, logger=CLIENT_LOGGER)
    env = {
        **scripted_engine_env(test_env, product, ATTACHED),
        "GOODFETCH_URL": (
            f"ws://user:{CANARY.value}@127.0.0.1:7878/?token={CANARY.value}"
        ),
        "GOODFETCH_TOKEN": "t",
    }

    error = await failure(open_collie(way, env))

    assert isinstance(error, SettingInvalidError)
    assert error.details["key"] == "GOODFETCH_URL"
    assert search_canaries_in([shown(error), logs(caplog)]) == []


@pytest.mark.covers("story:11-5", "error:setting_invalid")
@pytest.mark.parametrize(
    ("variables", "key"),
    [
        pytest.param(
            {"GOODFETCH_URL": "ws://192.0.2.1:7878", "GOODFETCH_TOKEN": "t"},
            "GOODFETCH_URL",
            id="names-no-loopback-address",
        ),
        pytest.param(
            {"GOODFETCH_URL": "https://127.0.0.1:7878", "GOODFETCH_TOKEN": "t"},
            "GOODFETCH_URL",
            id="is-no-ws-address",
        ),
        pytest.param(
            {"GOODFETCH_URL": "ws://127.0.0.1:7878"},
            "GOODFETCH_TOKEN",
            id="comes-without-goodfetch-token",
        ),
        pytest.param(
            {"GOODFETCH_URL": "ws://127.0.0.1:7878", "GOODFETCH_TOKEN": ""},
            "GOODFETCH_TOKEN",
            id="comes-with-an-empty-goodfetch-token",
        ),
        # The URL is judged first, as the CLI judges it.
        pytest.param(
            {"GOODFETCH_URL": "https://127.0.0.1:7878"},
            "GOODFETCH_URL",
            id="is-no-ws-address-and-comes-without-goodfetch-token",
        ),
        pytest.param(
            {"GOODFETCH_URL": "ws://127.0.0.1/path", "GOODFETCH_TOKEN": "t"},
            "GOODFETCH_URL",
            id="has-no-port",
        ),
    ],
)
@pytest.mark.anyio
async def test_fails_with_setting_invalid_before_any_engine_starts_when_goodfetch_url(
    variables: Mapping[str, str], key: str, test_env: TestEnv, product: ToolProduct
) -> None:
    """A refused `GOODFETCH_URL` or missing token, named by its key."""
    env = {**scripted_engine_env(test_env, product, ATTACHED), **variables}

    error = await failure(open_collie(Way.ASYNCIO, env))

    assert isinstance(error, SettingInvalidError)
    assert error.details["key"] == key


@pytest.mark.anyio
async def test_refuses_an_engine_choice_it_does_not_know(
    test_env: TestEnv, product: ToolProduct
) -> None:
    """A value outside `auto`, `attached`, `daemon` and `RemoteDaemon` is refused."""
    env = scripted_engine_env(test_env, product, ATTACHED)

    error = await failure(open_collie(Way.ASYNCIO, env, engine="cloud"))  # type: ignore[arg-type]

    assert isinstance(error, SettingInvalidError)
    assert error.details["key"] == "engine"


@pytest.mark.covers("story:11-4")
@pytest.mark.anyio
async def test_engine_daemon_starts_the_daemon_when_none_runs_and_reaches_it(
    env: Mapping[str, str], test_env: TestEnv, product: ToolProduct
) -> None:
    """`daemon` runs `daemon start` of the Product's executable, then reaches it."""
    try:
        collie = await open_collie(Way.ASYNCIO, env, engine="daemon")
        engine = collie.engine
        await collie.close()
    finally:
        stop = test_env.start(
            [test_env.tools.path(product.tool), "daemon", "stop"], env=env
        )
        stopped = await exit_code(lambda: stop.wait(30))

    assert stopped == 0
    assert (engine.kind, engine.product, engine.home) == (
        "daemon",
        Product.collie,
        str(test_env.home),
    )
