"""Collie's Connection on the Scripted engine (ADR 0069).

Each test plays a script of the shared corpus over the in-memory transport,
as an Attached engine's stdio would carry it, so the hand-written runtime
runs with no process, three ways: async on asyncio, async on Trio and
through the sync `Collie`. The script fails the test on a message it does
not expect, `hello` included, and on a step left unused. TypeScript's
`collie.test.ts` is its twin.
"""

import threading
from collections.abc import Awaitable, Mapping
from concurrent.futures import ThreadPoolExecutor
from datetime import timedelta
from typing import cast

import anyio
import pytest
import trio.testing
from collie_tracer import start_and_finish
from connection_library import finish_one_run
from goodfetch_collie import (
    AsyncCollie,
    AsyncConnection,
    Collie,
    Connection,
    DeadlineExceededError,
    EngineExitedError,
    EngineInfo,
    GoodfetchError,
    NotInProductError,
    Product,
    ProtocolVersionUnsupportedError,
    SettingInvalidError,
    StartedAs,
)
from goodfetch_collie.testing import async_connect_over, connect_over
from goodfetch_test_support import (
    Closed,
    Exited,
    Finished,
    PlayedMessage,
    PlayedScript,
    Way,
    play_script,
    three_ways,
)


async def collie_on(way: Way, script: str) -> tuple[AsyncCollie, PlayedScript]:
    """The Scripted engine playing `script`, and a Connection this way opened on it."""
    played = play_script(f"sdk-python/{script}")
    collie = await way.open(
        lambda: async_connect_over(played.transport),
        lambda: connect_over(played.transport),
    )
    return collie, played


@pytest.mark.covers("story:11-179")
@pytest.mark.parametrize(
    ("base", "product", "listed"),
    [
        pytest.param(
            AsyncConnection,
            AsyncCollie,
            {
                "call",
                "close",
                "engine",
                "explain_settings",
                "start",
                "__aenter__",
                "__aexit__",
            },
            id="async",
        ),
        pytest.param(
            Connection,
            Collie,
            {
                "call",
                "close",
                "engine",
                "explain_settings",
                "start",
                "__enter__",
                "__exit__",
            },
            id="sync",
        ),
    ],
)
def test_extends_the_connection_both_products_share_with_each_of_its_members(
    base: type[object], product: type[object], listed: set[str]
) -> None:
    """Collie's Connection extends the client package's, carrying each member.

    `listed` is what both Products' Connections share, the scoped form's two
    members included, as TypeScript's `Symbol.asyncDispose` is.
    """
    declared = {
        name
        for name, member in vars(base).items()
        if callable(member) or isinstance(member, property)
    }
    carried = {name for name in listed if hasattr(product, name)}

    assert (issubclass(product, base), declared, carried) == (True, listed, listed)


@pytest.mark.covers("story:11-54", "story:11-179")
@three_ways
async def test_starts_a_run_finishes_it_and_answers_its_run_report(way: Way) -> None:
    """The tracer program runs the same on asyncio, on Trio and through the sync API."""
    collie, played = await collie_on(way, "run/started-and-finished.json")

    traced = await start_and_finish(collie)

    assert traced.started_as == StartedAs.new
    assert traced.run_id == "01ke6p4ym0j8x3v5wq2rtbn7hc"
    # A result is built unvalidated (ADR 0082): its fields hold the engine's values.
    assert traced.report.model_dump(
        mode="json", exclude_unset=True, warnings=False
    ) == {
        "run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc",
        "lifecycle": "finished",
        "verdict": "verified",
        "completeness": "complete",
        "started_at": "2026-01-01T00:00:00Z",
        "ended_at": "2026-01-01T00:00:00Z",
        "usage": {
            "model": {
                "calls": 0,
                "tokens": 0,
                "input_tokens": 0,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0,
                "output_tokens": 0,
                "reasoning_tokens": 0,
                "price": None,
            },
            "browser_time": 0,
            "proxy_bytes": 0,
            "requests": 0,
        },
    }
    assert played.done() == Finished()


@pytest.mark.covers("story:11-180")
@three_ways
async def test_a_library_on_the_client_package_range_takes_the_programs_connection(
    way: Way,
) -> None:
    """A library that takes the client package by a range runs on its copy.

    `connection_library`'s helper, typed for the client package's
    `AsyncConnection`, finishes a Run on the program's Connection.
    """
    collie, played = await collie_on(way, "run/started-and-finished.json")

    report = await finish_one_run(collie)

    assert not isinstance(report, str)
    assert report.lifecycle == "finished"
    await collie.close()
    assert played.done() == Finished()


@pytest.mark.covers("story:11-180")
@three_ways
async def test_a_library_on_the_client_package_range_catches_its_closed_error(
    way: Way,
) -> None:
    """The library's own `except ClosedError` catches what a closed Connection raises.

    It holds only while the program and the library share one copy of the
    client package.
    """
    collie, played = await collie_on(way, "connect/accepted-by-foxhound.json")
    await collie.close()

    answer = await finish_one_run(collie)

    assert answer == "closed"
    assert played.done() == Finished()


@pytest.mark.covers("story:11-28", "story:11-40", "story:11-42")
@three_ways
async def test_takes_a_timeout_as_a_timedelta_only(way: Way) -> None:
    """`timeout=30_000` is a type error, and refused before it is sent (ADR 0080).

    mypy's `warn_unused_ignores` fails the check once the ignore below
    silences nothing, so the line is the type test.
    """
    collie, played = await collie_on(way, "run/started-and-finished.json")
    run = await collie.start()

    refused = await failure(run.finish(timeout=30_000))  # type: ignore[arg-type]
    await run.finish()
    await collie.close()

    assert (
        type(refused),
        refused.limit and refused.limit.model_dump(mode="json", exclude_none=True),
        "`timeout=None`" in (refused.fix or ""),
    ) == (
        SettingInvalidError,
        {"key": "timeout.run", "value": 30_000, "layer": "run", "origin": "argument"},
        True,
    )
    assert played.done() == Finished()


@three_ways
async def test_sends_each_scopes_settings_by_segment_durations_in_milliseconds(
    way: Way,
) -> None:
    """Starting a Run, a Session and a page each send their own settings (ADR 0083).

    A `timedelta` goes in whole milliseconds and `None` as null, which
    clears; `run/settings-sent-by-scope.json` expects each.
    """
    collie, played = await collie_on(way, "run/settings-sent-by-scope.json")
    run = await collie.start(
        settings={
            "timeout": {"navigation": timedelta(seconds=30)},
            "checkpoint": {"interval": None},
        }
    )
    session = await run.new_session(settings={"browser": {"access_mode": "stealth"}})
    await session.new_page(settings={"content": {"extractor": "trafilatura"}})
    await collie.close()

    assert played.done() == Finished()


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


async def refusal(way: Way, script: str) -> tuple[GoodfetchError, PlayedScript]:
    """The error opening a Connection this way over `script` raises."""
    played = play_script(f"sdk-python/{script}")
    error = await failure(
        way.open(
            lambda: async_connect_over(played.transport),
            lambda: connect_over(played.transport),
        )
    )
    return error, played


def sent(played: PlayedScript, method: str) -> bool:
    """Whether the Client has sent a request for `method`."""
    return any(
        isinstance(played_message, PlayedMessage)
        and played_message.from_ == "client"
        and isinstance(message := played_message.message, Mapping)
        and cast("Mapping[str, object]", message).get("method") == method
        for played_message in played.messages
    )


async def until_sent(played: PlayedScript, method: str) -> None:
    """Waits until the Client has sent a request for `method`, for 5 seconds at most.

    The Scripted engine's player records each message as it reads it, and
    sets no event, so the wait polls it.
    """
    for _ in range(5000):
        if sent(played, method):
            return
        await anyio.sleep(0.001)
    message = f"the Client sent no {method} within 5 seconds"
    raise AssertionError(message)


@three_ways
async def test_reports_the_engine_it_reached_of_either_product(way: Way) -> None:
    """A Foxhound engine serves a Collie Connection, and says so (ADR 0095)."""
    collie, played = await collie_on(way, "connect/accepted-by-foxhound.json")

    engine = collie.engine
    await collie.close()

    assert engine == EngineInfo(
        kind="attached",
        product=Product.foxhound,
        version="0.0.0",
        protocol_version="0.1",
        project_file=None,
        home=None,
        engine_path=None,
        url=None,
        fallback=None,
    )
    assert played.done() == Finished()


@pytest.mark.covers("story:11-54")
@three_ways
async def test_fails_connect_when_the_engine_refuses_its_product_naming_the_fix(
    way: Way,
) -> None:
    """`not_in_product` raises from `connect()`, with the engine's `fix`."""
    error, played = await refusal(way, "connect/refused-not-in-product.json")

    assert isinstance(error, NotInProductError)
    assert (error.code, error.group) == ("not_in_product", "unavailable")
    assert error.fix == "Install goodfetch-collie and connect to its engine."
    assert error.details == {
        "needs": "collie",
        "product": "foxhound",
        "product_version": "0.0.0",
    }
    assert played.done() == Closed()


@three_ways
async def test_fails_connect_when_the_engine_refuses_its_protocol_version(
    way: Way,
) -> None:
    """The engine's `protocol_version_unsupported` raises from `connect()`."""
    error, played = await refusal(
        way, "connect/refused-protocol-version-unsupported.json"
    )

    assert isinstance(error, ProtocolVersionUnsupportedError)
    assert error.fix == (
        "Upgrade the engine to a version that speaks protocol 0.1, "
        "or use a Client of the engine's own version."
    )
    assert played.done() == Closed()


@three_ways
@pytest.mark.parametrize(
    ("script", "engine_protocol_version", "fix"),
    [
        pytest.param(
            "connect/answered-with-another-major.json",
            "1.0",
            "Use the engine and the SDK of one Product version: this SDK 0.0.0 "
            "speaks protocol 0.1, and the engine 1.0.0 speaks 1.0.",
            id="another-major",
        ),
        pytest.param(
            "connect/answered-with-another-preview-minor.json",
            "0.2",
            "Use the engine and the SDK of one Product version: this SDK 0.0.0 "
            "speaks protocol 0.1, and the engine 0.2.0 speaks 0.2.",
            id="newer-preview-minor",
        ),
        pytest.param(
            "connect/answered-without-a-protocol-version.json",
            None,
            "Use the engine and the SDK of one Product version: this SDK 0.0.0 "
            "speaks protocol 0.1.",
            id="no-protocol-version",
        ),
    ],
)
async def test_refuses_an_engine_of_another_protocol_line(
    way: Way, script: str, engine_protocol_version: str | None, fix: str
) -> None:
    """A newer major, a newer preview minor or no version: the Client refuses it."""
    error, played = await refusal(way, script)

    assert isinstance(error, ProtocolVersionUnsupportedError)
    assert error.details == {
        "client_protocol_version": "0.1",
        "engine_protocol_version": engine_protocol_version,
    }
    assert error.fix == fix
    assert played.done() == Finished()


@three_ways
async def test_aborts_a_run_once_when_two_closes_run_at_once(way: Way) -> None:
    """Every close waits for the one close, which aborts the Run still running once."""
    collie, played = await collie_on(way, "run/closed-while-running.json")
    await collie.start(run_id="nightly", tags=["shop"])

    async with anyio.create_task_group() as closes:
        closes.start_soon(collie.close)
        closes.start_soon(collie.close)

    assert played.done() == Finished()


@pytest.mark.covers(
    "story:11-41", "story:11-42", "setting:timeout.connect", "error:deadline_exceeded"
)
@pytest.mark.anyio
@pytest.mark.parametrize(
    "anyio_backend",
    # Trio's clock jumps ahead whenever every task waits; the sync way's
    # private loop keeps real time.
    [
        pytest.param(
            ("trio", {"clock": trio.testing.MockClock(autojump_threshold=0)}), id="trio"
        )
    ],
)
async def test_fails_the_connect_once_timeout_connects_default_passes() -> None:
    """With `timeout` left out, an unanswered `hello` fails it after 30 s."""
    played = play_script("sdk-python/connect/hello-never-answered.json")

    error = await failure(async_connect_over(played.transport))

    assert isinstance(error, DeadlineExceededError)
    assert (error.code, error.group) == ("deadline_exceeded", "deadline_exceeded")
    assert error.limit is not None
    assert error.limit.model_dump(mode="json", exclude_none=True) == {
        "key": "timeout.connect",
        "value": 30_000,
        "layer": "default",
    }
    assert error.details == {
        "elapsed": 30_000,
        "waited_for": "hello",
    }
    assert played.done() == Finished()


@pytest.mark.anyio
@pytest.mark.parametrize(
    "anyio_backend",
    # Trio's clock jumps ahead whenever every task waits; the sync way's
    # private loop keeps real time.
    [
        pytest.param(
            ("trio", {"clock": trio.testing.MockClock(autojump_threshold=0)}), id="trio"
        )
    ],
)
async def test_gives_up_on_a_runs_abort_once_timeout_close_passes() -> None:
    """Past `timeout.close`, the close kills the engine: `deadline_exceeded`."""
    played = play_script("sdk-python/run/abort-never-answered.json")
    collie = await async_connect_over(played.transport)
    await collie.start(run_id="nightly")

    error = await failure(collie.close())

    assert isinstance(error, DeadlineExceededError)
    assert (error.code, error.group) == ("deadline_exceeded", "deadline_exceeded")
    assert error.limit is not None
    assert error.limit.model_dump(mode="json", exclude_none=True) == {
        "key": "timeout.close",
        "value": 30_000,
        "layer": "default",
    }
    assert error.details == {
        "elapsed": 30_000,
        "waited_for": "close",
    }
    assert played.done() == Finished()


@three_ways
async def test_fails_the_call_in_flight_when_the_engine_ends_first(way: Way) -> None:
    """The call fails with `engine_exited`, not retryable."""
    collie, played = await collie_on(way, "connect/exit-during-a-call.json")

    error = await failure(collie.start())
    await collie.close()

    assert isinstance(error, EngineExitedError)
    assert (error.code, error.group, error.retryable) == (
        "engine_exited",
        "unavailable",
        False,
    )
    assert error.details == {"exit_code": None, "signal": None, "stderr_tail": ""}
    assert played.done() == Exited(101)


@pytest.mark.covers("story:11-54")
@pytest.mark.anyio
@pytest.mark.parametrize("anyio_backend", ["asyncio"])
async def test_the_sync_collie_runs_inside_a_running_event_loop() -> None:
    """As in a notebook cell: the sync API never touches the caller's loop."""
    played = play_script("sdk-python/run/started-and-finished.json")

    with connect_over(played.transport) as collie:
        run = collie.start()
        report = run.finish()

    assert report.lifecycle == "finished"
    assert played.done() == Finished()


@pytest.mark.covers("story:11-54")
def test_threads_share_one_collie_and_its_runs() -> None:
    """A Collie and its Run serve any thread, and the close joins its private thread."""
    played = play_script("sdk-python/run/started-and-finished.json")
    # Only this Collie's thread: one an earlier test left open is not its own.
    earlier = set(threading.enumerate())
    collie = connect_over(played.transport)
    private = {
        thread
        for thread in threading.enumerate()
        if thread.name == "goodfetch-collie" and thread not in earlier
    }

    with ThreadPoolExecutor(4) as pool:
        run = pool.submit(collie.start).result()
        report = pool.submit(run.finish).result()
        for closed in [pool.submit(collie.close) for _ in range(4)]:
            closed.result()

    assert report.lifecycle == "finished"
    assert private
    assert not any(thread.is_alive() for thread in private)
    assert played.done() == Finished()


async def connect_failure(way: Way, env: Mapping[str, str]) -> GoodfetchError:
    """The error `connect()` raises this way with `env`, before any engine starts."""
    return await failure(
        way.open(
            lambda: AsyncCollie.connect(env=env),
            lambda: Collie.connect(env=env),
        )
    )


def summary(error: GoodfetchError) -> dict[str, object]:
    """What a program reads of `error`."""
    return {
        "code": error.code,
        "group": error.group,
        "fix": error.fix,
        "details": error.details,
    }


@pytest.mark.covers("story:11-2", "error:engine_not_found")
@three_ways
async def test_fails_connect_when_its_engine_path_names_no_executable(
    way: Way,
) -> None:
    """The `fix` names the variable and the executable it should point at."""
    path = "/nonexistent/goodfetch-collie"

    error = await connect_failure(way, {"GOODFETCH_COLLIE_ENGINE_PATH": path})

    assert summary(error) == {
        "code": "engine_not_found",
        "group": "unavailable",
        "fix": (
            f"GOODFETCH_COLLIE_ENGINE_PATH names {path}, which is no executable "
            "file: point it at Collie's executable, goodfetch-collie, or leave it "
            "unset to use the one goodfetch-collie installed."
        ),
        "details": {"path": path},
    }


@pytest.mark.covers("story:11-2", "error:engine_not_found")
@three_ways
async def test_fails_connect_when_no_platform_wheel_is_installed_naming_it(
    way: Way,
) -> None:
    """With the variable unset, the `fix` names the platform wheel to install."""
    error = await connect_failure(way, {})

    assert (error.code, error.group, error.fix) == (
        "engine_not_found",
        "unavailable",
        "Install goodfetch-collie's platform wheel for this system, which holds "
        "its engine, or set GOODFETCH_COLLIE_ENGINE_PATH to Collie's executable.",
    )


@pytest.mark.covers("story:11-2", "story:11-73", "error:setting_invalid")
@pytest.mark.parametrize("variable", ["GOODFETCH_COLLIE_ENGINE_PATH", "GOODFETCH_URL"])
@three_ways
async def test_fails_connect_when_a_deployment_variable_is_set_but_empty(
    way: Way, variable: str
) -> None:
    """An empty value is never taken for unset (ADR 0083)."""
    error = await connect_failure(way, {variable: ""})

    assert summary(error) == {
        "code": "setting_invalid",
        "group": "program",
        "fix": f"Give {variable} a value, or unset it.",
        "details": {"key": variable, "reason": "it is empty; unset it instead"},
    }


@pytest.mark.covers("story:11-73", "error:setting_invalid")
@three_ways
async def test_fails_connect_when_a_deployment_variable_is_no_unicode(
    way: Way,
) -> None:
    """A byte `os.environ` could not decode is refused, not passed on."""
    error = await connect_failure(way, {"GOODFETCH_HOME": "/home/\udcff"})

    assert summary(error)["details"] == {
        "key": "GOODFETCH_HOME",
        "reason": "it is not Unicode",
    }


@pytest.mark.covers("story:11-73", "error:setting_unknown")
@three_ways
async def test_fails_connect_on_a_goodfetch_variable_off_the_list(
    way: Way,
) -> None:
    """The `GOODFETCH_` prefix is reserved: the closest variable is named."""
    error = await connect_failure(way, {"GOODFETCH_HOEM": "/home/goodfetch"})

    assert str(error) == (
        "`GOODFETCH_HOEM` in the environment is no deployment variable; "
        "did you mean `GOODFETCH_HOME`?"
    )
    assert summary(error) == {
        "code": "setting_unknown",
        "group": "program",
        "fix": "Rename GOODFETCH_HOEM to GOODFETCH_HOME, or unset it.",
        "details": {"key": "GOODFETCH_HOEM", "closest": "GOODFETCH_HOME"},
    }


@pytest.mark.covers("story:11-73", "error:setting_unknown")
@three_ways
async def test_fails_connect_on_a_goodfetch_variable_close_to_none_naming_none(
    way: Way,
) -> None:
    """A name far from every deployment variable gets no `closest`."""
    error = await connect_failure(way, {"GOODFETCH_TELEMETRY": "1"})

    assert error.code == "setting_unknown"
    assert error.details == {"key": "GOODFETCH_TELEMETRY"}


@pytest.mark.covers("story:11-2", "error:setting_invalid")
@three_ways
async def test_fails_connect_when_its_listener_queue_takes_no_event(
    way: Way,
) -> None:
    """A bound under 1 fails before any engine starts, naming the option."""
    error = await failure(
        way.open(
            lambda: AsyncCollie.connect(env={}, max_listener_queue=0),
            lambda: Collie.connect(env={}, max_listener_queue=0),
        )
    )

    assert summary(error) == {
        "code": "setting_invalid",
        "group": "program",
        "fix": (
            "Pass max_listener_queue a whole number of 1 or more, or leave it "
            "out for its default of 1024."
        ),
        "details": {
            "key": "max_listener_queue",
            "reason": "0 is no whole number of 1 or more",
        },
    }


@pytest.mark.anyio
@pytest.mark.covers("story:11-181")
async def test_cancels_a_raw_call_with_the_task_it_runs_in() -> None:
    """Cancelling its task cancels the raw call: `$/cancel` reaches the engine."""
    played = play_script("sdk-python/connect/raw-call-cancelled.json")
    collie = await async_connect_over(played.transport)

    async with anyio.create_task_group() as calls:
        calls.start_soon(collie.call, "run.list", {"lifecycle": "running"})
        await until_sent(played, "run.list")
        calls.cancel_scope.cancel()
    await collie.close()

    assert played.done() == Finished()
