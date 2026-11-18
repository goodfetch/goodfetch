"""Limits in code on an Attached engine (ADRs 0041, 0080 and 0102).

A program caps its Connection at `connect()` and a Run at its start, with a
price as an exact `Money`, and each only lowers what covers that work. The
cases play on the Scripted engine started as the executable, which checks
every message the SDK sends, three ways: async on asyncio, async on Trio and
through the sync `Collie`; the last test runs the same calls on the
executable of the same commit. TypeScript's `limits.test.ts` is its twin.
"""

from collections.abc import Awaitable, Mapping
from decimal import Decimal

import pytest
from goodfetch_collie import (
    AsyncCollie,
    Collie,
    GoodfetchError,
    LimitRaiseRefusedError,
    LimitReachedError,
    Limits,
    Money,
    Prices,
    SettingInvalidError,
    StartedAs,
)
from goodfetch_collie_client.generated.models import LimitScope, StopReason6
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

ENGINE_LOGGER = "goodfetch.client"
"""The logger the engine's stderr lines go to."""


async def open_collie(
    way: Way, env: Mapping[str, str], limits: Limits | None = None
) -> AsyncCollie:
    """A Connection this way opened on the Attached engine `env` names."""
    return await way.open(
        lambda: AsyncCollie.connect(env=env, limits=limits),
        lambda: Collie.connect(env=env, limits=limits),
    )


def engine_warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
    """Each line the engines wrote on stderr, which a failed play writes."""
    return [
        record.getMessage() for record in caplog.records if record.name == ENGINE_LOGGER
    ]


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


@pytest.mark.covers("story:11-51")
@pytest.mark.client_case(id="limits-sent-in-millionths", mode="attached")
@three_ways
async def test_sends_a_connections_limits_after_hello_and_a_runs_in_its_start(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Each count goes as given, each price in millionths of its currency."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/limits-sent-in-millionths.json"
    )
    collie = await open_collie(
        way,
        env,
        Limits(
            model_calls=100, price=Prices(paid_solves=Money(Decimal("2.50"), "USD"))
        ),
    )
    run = await collie.start(
        run_id="capped",
        limits=Limits(
            browser_time=60_000,
            price=Prices(proxy_bytes=Money(Decimal("0.000001"), "EUR")),
        ),
    )

    report = await run.finish()
    await collie.close()

    with client_case.check("the Run started and finished under its Limits"):
        assert (report.run_id.root, report.lifecycle) == ("capped", "finished")
    with client_case.check(
        "the engine read each Limit as the script expects, so it wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-51")
@pytest.mark.client_case(id="money-inexact-refused-before-sending", mode="attached")
@three_ways
async def test_refuses_a_money_it_cannot_turn_exactly_before_sending_anything(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A float, a malformed amount and a sub-millionth fail from the start."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/money-inexact-refused.json"
    )
    collie = await open_collie(way, env)

    finer = await failure(
        collie.start(
            limits=Limits(price=Prices(proxy_bytes=Money(Decimal("0.0000001"), "USD")))
        )
    )
    malformed = await failure(
        collie.start(
            limits=Limits(price=Prices(model_tokens=Money(Decimal("NaN"), "USD")))
        )
    )
    floated = await failure(
        collie.start(
            limits=Limits(
                price=Prices(paid_solves=Money(0.5, "USD"))  # type: ignore[arg-type]
            )
        )
    )
    await collie.close()

    with client_case.check(
        "an amount finer than a millionth fails with setting_invalid"
    ):
        assert isinstance(finer, SettingInvalidError)
        assert (finer.code, finer.group, finer.details["key"]) == (
            "setting_invalid",
            "program",
            "limits.price.proxy_bytes",
        )
    with client_case.check("an amount that is no decimal fails with setting_invalid"):
        assert (malformed.code, malformed.details["key"]) == (
            "setting_invalid",
            "limits.price.model_tokens",
        )
    with client_case.check("a float fails with setting_invalid"):
        assert (floated.code, floated.details["key"]) == (
            "setting_invalid",
            "limits.price.paid_solves",
        )
    with client_case.check("nothing reached the engine past hello"):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-51")
@pytest.mark.client_case(id="connect-limit-refused-closes", mode="attached")
@three_ways
async def test_fails_connect_when_the_engine_refuses_one_of_its_limits(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The refusal fails `connect()`, which ends the engine it started."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/connect/limit-refused.json"
    )

    refused = await failure(
        open_collie(
            way, env, Limits(price=Prices(paid_solves=Money(Decimal(2), "USD")))
        )
    )

    with client_case.check("connect() fails with the engine's refusal"):
        assert refused.code == "invalid_params"
    with client_case.check("the engine was ended, so its play wrote no failure"):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-51", "error:limit_raise_refused")
@pytest.mark.client_case(id="limits-lowered-never-raised", mode="attached")
@three_ways
async def test_lowers_a_resumed_runs_limit_and_never_raises_it(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A lower Limit lowers it, an equal one stands, a higher one is refused."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/limits-lowered-never-raised.json"
    )
    collie = await open_collie(way, env)

    started_as: list[StartedAs] = []
    for requests in (5, 3, 5):
        run = await collie.start(run_id="nightly", limits=Limits(requests=requests))
        started_as.append(run.started_as)
        await run.close()
    raised = await failure(collie.start(run_id="nightly", limits=Limits(requests=8)))
    await collie.close()

    with client_case.check("each start below or at the Limit resumed the Run"):
        assert started_as == [StartedAs.new, StartedAs.resumed, StartedAs.resumed]
    with client_case.check("a start above it fails with limit_raise_refused"):
        assert isinstance(raised, LimitRaiseRefusedError)
        assert (raised.code, raised.group, raised.retryable, raised.details) == (
            "limit_raise_refused",
            "refused",
            False,
            {"usage_kind": "requests", "given": 8, "standing": 5},
        )
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-52", "error:limit_reached")
@pytest.mark.client_case(id="limit-reached-stops-the-run", mode="attached")
@three_ways
async def test_fails_the_call_that_finds_a_limit_used_up_and_the_run_ends_stopped(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The call raises `limit_reached`; the Run ends `stopped` and `incomplete`."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/limit-reached-stops-the-run.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start(
        run_id="capped",
        limits=Limits(price=Prices(proxy_bytes=Money(Decimal(1), "USD"))),
    )
    page = await run.new_page()

    reached = await failure(page.goto("https://shop.test/"))
    report = await run.finish()
    await collie.close()

    with client_case.check("the call fails with limit_reached, naming the Limit"):
        assert isinstance(reached, LimitReachedError)
        assert (
            reached.code,
            reached.group,
            reached.details["scope"],
            reached.details["usage_kind"],
            reached.details["currency"],
        ) == (
            "limit_reached",
            "refused",
            {"kind": "run", "id": "capped"},
            "proxy_bytes",
            "USD",
        )
    with client_case.check("the Run ends stopped and incomplete, for that Limit"):
        assert (report.lifecycle, report.completeness) == ("stopped", "incomplete")
        assert report.stop_reason is not None
        stopped_by = report.stop_reason.root
        assert isinstance(stopped_by, StopReason6)
        assert (
            stopped_by.scope,
            stopped_by.usage_kind,
            stopped_by.currency,
        ) == (LimitScope(kind="run", id="capped"), "proxy_bytes", "USD")
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-51")
@three_ways
async def test_holds_a_connections_limits_and_lowers_a_resumed_runs_on_the_executable(
    way: Way, env: Mapping[str, str]
) -> None:
    """The engine of the same commit enforces both, and refuses a raise."""
    capped = await open_collie(way, env, Limits(requests=0))
    reached = await failure(capped.start())
    await capped.close()
    collie = await open_collie(
        way,
        env,
        Limits(
            model_calls=100, price=Prices(paid_solves=Money(Decimal("2.50"), "USD"))
        ),
    )

    started_as: list[StartedAs] = []
    for requests in (5, 3, 5):
        run = await collie.start(run_id="resumed", limits=Limits(requests=requests))
        started_as.append(run.started_as)
        await run.close()
    raised = await failure(collie.start(run_id="resumed", limits=Limits(requests=8)))
    await collie.close()

    assert isinstance(reached, LimitReachedError)
    assert LimitScope.model_validate(reached.details["scope"]).kind == "connection"
    assert started_as == [StartedAs.new, StartedAs.resumed, StartedAs.resumed]
    assert (raised.code, raised.details["usage_kind"], raised.details["given"]) == (
        "limit_raise_refused",
        "requests",
        8,
    )
