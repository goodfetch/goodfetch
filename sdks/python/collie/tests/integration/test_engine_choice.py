"""Which engine `connect()` reaches (ADRs 0024, 0078 and 0083).

With `engine` left at `auto`, the remote Daemon `GOODFETCH_URL` names, else
the Daemon whose runtime file is in the Goodfetch home, else an Attached
engine; `attached`, `daemon` and `RemoteDaemon(url, token)` choose one,
beating `GOODFETCH_URL`. Each plays a Scripted engine's script (ADR 0069),
as an Attached engine, as a Daemon on a Unix socket or on loopback TCP, or
as a remote Daemon's WebSocket. A
Scripted Daemon exits 0 only once every step played, so its exit says the
Client sent exactly what the script holds, the Daemon's token in `hello`
included. A Daemon that refuses `hello` by Product or protocol version is
left for an Attached engine under `auto`, waited past when it hands over,
and raised under `daemon` or when remote (ADR 0098).
`test_engine_choice_daemon.py` reaches the Daemon of the same commit.
TypeScript's `engine-choice.test.ts` is its twin.
"""

import json
import logging
from collections.abc import Mapping
from datetime import timedelta
from pathlib import Path

import anyio
import anyio.to_thread
import pytest
from collie_calls import (
    CLIENT_LOGGER,
    engine_warnings,
    exit_code,
    failure,
    open_collie,
)
from goodfetch_collie import (
    AsyncCollie,
    DaemonUnreachableError,
    DeadlineExceededError,
    EngineChoice,
    EngineFallback,
    EngineInfo,
    EngineNotFoundError,
    GoodfetchError,
    Product,
    ProtocolVersionUnsupportedError,
    RemoteDaemon,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.connection import WEBSOCKET_SUBPROTOCOL
from goodfetch_test_support import (
    Canary,
    CanaryHit,
    ClientCaseRecord,
    NamedOutput,
    ScriptedDaemon,
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


def start_remote_daemon(
    test_env: TestEnv, script: str, subprotocol: str = WEBSOCKET_SUBPROTOCOL
) -> ScriptedDaemon:
    """A Scripted Daemon serving an Exposed Daemon's WebSocket (ADR 0049).

    It answers `subprotocol` when the handshake offers it: the SDK's own, or
    another major's to play a Daemon of another protocol major.
    """
    return start_scripted_daemon(test_env, script, "websocket", subprotocol=subprotocol)


def write_runtime_file(home: Path, contents: str) -> None:
    """Writes a runtime file in `home`'s per-major folder, as a Daemon publishes one.

    `0.0` is the folder of the Scripted engine's version 0.0.0 (ADR 0030).
    """
    folder = home / "0.0"
    folder.mkdir(parents=True, exist_ok=True)
    file = folder / "daemon.json"
    file.write_text(contents, "utf-8")
    file.chmod(0o600)


def logged(caplog: pytest.LogCaptureFixture) -> list[tuple[int, str, object]]:
    """Each line `goodfetch.client` got: its level, its message and its fields."""
    return [
        (
            record.levelno,
            record.getMessage(),
            {
                field: record.__dict__[field]
                for field in ("fallback", "engine")
                if field in record.__dict__
            },
        )
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
        [(level, message, fields)] = logged(caplog)
        assert (level, fields) == (logging.INFO, {"engine": engine})
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
        [(level, message, fields)] = logged(caplog)
        assert (level, fields) == (logging.INFO, {"engine": engine})
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
    """`GOODFETCH_URL` over a WebSocket of subprotocol `goodfetch.v<major>`.

    With `GOODFETCH_TOKEN`'s canary token.
    """
    caplog.set_level(logging.DEBUG, logger=CLIENT_LOGGER)
    daemon = start_remote_daemon(test_env, REMOTE)
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
    daemon = start_remote_daemon(test_env, REMOTE)
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
async def test_fails_with_daemon_unreachable_when_no_offered_subprotocol_is_answered(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """A Daemon of another protocol major is never sent `hello`."""
    # A Daemon of another protocol major answers its own subprotocol only.
    daemon = start_remote_daemon(test_env, REMOTE, "goodfetch.v9999")
    url = f"ws://{daemon.address}"
    env = scripted_engine_env(test_env, product, ATTACHED)

    error = await failure(open_collie(way, env, engine=RemoteDaemon(url, CANARY.value)))
    # The script expected `hello`: the Scripted engine fails its play.
    code = await exit_code(daemon.wait)

    assert code != 0
    assert isinstance(error, DaemonUnreachableError)
    assert error.group == "unavailable"
    assert url in (error.fix or "")
    assert WEBSOCKET_SUBPROTOCOL in str(error)


@pytest.mark.covers("story:11-5", "error:daemon_unreachable")
@three_ways
async def test_fails_with_daemon_unreachable_naming_goodfetch_token_on_close_at_hello(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """A Daemon that closes the Connection before it answers `hello`."""
    daemon = start_remote_daemon(test_env, REMOTE)
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


VERSION_REFUSED = "sdk-python/connect/daemon-refused-protocol-version-unsupported.json"
"""The script of a Daemon of an older protocol that refuses `hello` (ADR 0098)."""

VERSION_FIX = (
    "Upgrade the engine to a version that speaks protocol 0.1, or use a Client of"
    " the engine's own version."
)
"""The `fix` the refusal of `VERSION_REFUSED` gives."""

HANDS_OVER = "sdk-python/connect/daemon-hands-over.json"
"""The script of an idle Daemon that refuses `hello` and hands over to the pick."""


def restart_fix(path: str) -> str:
    """What a refusal's `fix` adds for a Collie Client whose executable is `path`.

    Its `daemon restart`, and Foxhound at its version or newer, since a
    Foxhound of the major would still be the pick (ADR 0098).
    """
    return (
        f"Run `{path} daemon restart` to restart the Daemon on its major's pick."
        " Collie's executable would still not run the Daemon while a Foxhound of"
        " the major is installed: install Foxhound 0.0.0 or newer (`pip install"
        " goodfetch-foxhound`) and run its `daemon restart`."
    )


@pytest.mark.covers("story:11-9", "story:11-290")
@pytest.mark.client_case(
    id="connect-falls-back-when-the-daemon-refuses", mode="attached"
)
@pytest.mark.parametrize(
    ("script", "fix", "code"),
    [
        (
            "sdk-python/connect/daemon-refused-not-in-product.json",
            "Install goodfetch-collie and connect to its engine.",
            "not_in_product",
        ),
        (VERSION_REFUSED, VERSION_FIX, "protocol_version_unsupported"),
    ],
    ids=["by-product", "by-protocol-version"],
)
@three_ways
async def test_falls_back_to_an_attached_engine_when_the_daemon_refuses_and_says_why(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
    script: str,
    fix: str,
    code: str,
) -> None:
    """With `engine` at `auto`, a Daemon refusing by Product or version is left."""
    caplog.set_level(logging.INFO, logger=CLIENT_LOGGER)
    env = scripted_engine_env(test_env, product, ATTACHED)
    daemon = start_scripted_daemon(test_env, script, "socket")

    collie = await open_collie(way, env)
    engine = collie.engine
    await collie.close()
    played = await exit_code(daemon.wait)

    path = env["GOODFETCH_COLLIE_ENGINE_PATH"]
    with client_case.check(
        "the Daemon played its script through: hello carried its token, and it"
        " refused it"
    ):
        assert played == 0
    with client_case.check(
        "engine reports the Attached engine, and the fallback's cause and fix"
    ):
        assert engine.fallback is not None
        assert code in engine.fallback.cause
        assert daemon.address in engine.fallback.cause
        assert engine == EngineInfo(
            kind="attached",
            product=Product.collie,
            version="0.0.0",
            protocol_version="0.1",
            project_file=None,
            home=str(test_env.home),
            engine_path=path,
            url=None,
            fallback=EngineFallback(
                cause=engine.fallback.cause, fix=f"{fix} {restart_fix(path)}"
            ),
        )
    with client_case.check(
        "the fallback is logged once at warning, before the engine reached at info"
    ):
        [(warning, why, fields), (info, reached_text, reached_fields)] = logged(caplog)
        assert (warning, fields) == (logging.WARNING, {"fallback": engine.fallback})
        assert daemon.address in why
        assert (info, reached_fields) == (logging.INFO, {"engine": engine})
        assert "Attached engine" in reached_text


@pytest.mark.covers("story:11-293")
@three_ways
async def test_names_the_install_in_the_refusals_fix_when_no_executable_is_there(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """With no executable to restart the Daemon with, the fix installs one."""
    missing = str(test_env.home / "no-collie-here")
    env = {
        **scripted_engine_env(test_env, product, ATTACHED),
        "GOODFETCH_COLLIE_ENGINE_PATH": missing,
    }
    daemon = start_scripted_daemon(test_env, VERSION_REFUSED, "socket")

    error = await failure(open_collie(way, env, engine="daemon"))

    assert await exit_code(daemon.wait) == 0
    assert isinstance(error, ProtocolVersionUnsupportedError)
    assert error.fix == (
        f"{VERSION_FIX} GOODFETCH_COLLIE_ENGINE_PATH names {missing}, which is no"
        " executable file: point it at Collie's executable, goodfetch-collie, or"
        " leave it unset to use the one goodfetch-collie installed."
    )


@pytest.mark.covers("story:11-9", "story:11-293", "error:protocol_version_unsupported")
@pytest.mark.client_case(id="connect-falls-back-when-the-daemon-refuses", mode="daemon")
@three_ways
async def test_fails_with_the_refusal_under_engine_daemon_its_fix_naming_restart(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """With `engine` at `daemon`, the refusal raises; its fix adds `daemon restart`."""
    caplog.set_level(logging.DEBUG, logger=CLIENT_LOGGER)
    env = scripted_engine_env(test_env, product, ATTACHED)
    daemon = start_scripted_daemon(test_env, VERSION_REFUSED, "socket")

    error = await failure(open_collie(way, env, engine="daemon"))
    played = await exit_code(daemon.wait)

    with client_case.check("the Daemon played its script through"):
        assert played == 0
    with client_case.check(
        "connect() raises the refusal, its fix naming this executable's daemon"
        " restart and Foxhound"
    ):
        assert isinstance(error, ProtocolVersionUnsupportedError)
        assert error.fix == (
            f"{VERSION_FIX} {restart_fix(env['GOODFETCH_COLLIE_ENGINE_PATH'])}"
        )
        assert error.details == {
            "client_protocol_version": "0.1",
            "engine_protocol_version": "0.0",
        }
    with client_case.check("nothing is logged, and no Attached engine started"):
        assert logged(caplog) == []


@pytest.mark.covers("story:11-295", "error:protocol_version_unsupported")
@pytest.mark.parametrize(
    "in_env", [False, True], ids=["remote-daemon", "goodfetch-url"]
)
@three_ways
async def test_fails_with_a_remote_daemons_refusal_which_it_never_falls_back_from(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
    in_env: bool,
) -> None:
    """Through `RemoteDaemon(url, token)` or `GOODFETCH_URL`, the refusal is raised."""
    caplog.set_level(logging.DEBUG, logger=CLIENT_LOGGER)
    daemon = start_remote_daemon(test_env, VERSION_REFUSED)
    url = f"ws://{daemon.address}"
    attached = scripted_engine_env(test_env, product, ATTACHED)
    env = (
        {**attached, "GOODFETCH_URL": url, "GOODFETCH_TOKEN": "scripted-daemon-token"}
        if in_env
        else attached
    )
    engine: EngineChoice = (
        "auto" if in_env else RemoteDaemon(url, "scripted-daemon-token")
    )

    error = await failure(open_collie(way, env, engine=engine))

    assert await exit_code(daemon.wait) == 0
    assert isinstance(error, ProtocolVersionUnsupportedError)
    assert error.fix == VERSION_FIX
    assert logged(caplog) == []


@pytest.mark.covers("story:11-292")
@three_ways
async def test_waits_for_the_pick_an_idle_daemon_hands_over_to_and_reaches_it(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A refusal marked `handover` is waited past until the pick serves."""
    caplog.set_level(logging.INFO, logger=CLIENT_LOGGER)
    env = scripted_engine_env(test_env, product, ATTACHED)
    handing = start_scripted_daemon(test_env, HANDS_OVER, "socket")
    opened: list[AsyncCollie] = []
    picks: list[ScriptedDaemon] = []

    async def connect() -> None:
        opened.append(await open_collie(way, env))

    async with anyio.create_task_group() as group:
        group.start_soon(connect)
        assert await exit_code(handing.wait) == 0
        # The pick starts once the Daemon that handed over has stopped.
        picks.append(
            await anyio.to_thread.run_sync(
                start_scripted_daemon, test_env, DAEMON, "socket"
            )
        )
    [collie] = opened
    [pick] = picks
    engine = collie.engine
    await collie.close()

    assert await exit_code(pick.wait) == 0
    assert (engine.kind, engine.url, engine.fallback) == ("daemon", pick.address, None)
    [(level, message, fields)] = logged(caplog)
    assert (level, fields) == (logging.INFO, {"engine": engine})
    assert "Daemon" in message


@pytest.mark.covers("story:11-292", "error:deadline_exceeded")
@three_ways
async def test_fails_with_deadline_exceeded_when_no_pick_follows_a_handover_in_time(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """Past `timeout.connect`, counted from the connect's start, no pick served."""
    env = scripted_engine_env(test_env, product, ATTACHED)
    handing = start_scripted_daemon(test_env, HANDS_OVER, "socket")

    error = await failure(
        open_collie(way, env, connect_timeout=timedelta(milliseconds=300))
    )

    assert await exit_code(handing.wait) == 0
    assert isinstance(error, DeadlineExceededError)
    assert error.limit is not None
    assert (
        error.limit.key,
        error.limit.value,
        error.limit.layer,
        error.limit.origin,
    ) == ("timeout.connect", 300, "run", "argument")
    assert error.details["waited_for"] == "hello"
    # Past the refusal, not on the first hello, whose fix names the engine.
    assert error.fix == (
        "Raise connect()'s timeout, or find why the Daemon did not start in its"
        f" log in {test_env.home}."
    )
