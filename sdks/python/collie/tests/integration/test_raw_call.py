"""The raw catalog call (ADR 0099, G8) on the Scripted engine as a Foxhound engine.

A Collie program reaches it: `call` on the Connection and on each value
sends the wire's JSON and answers the engine's unchanged, and the main
path's protections hold. TypeScript's `raw-call.test.ts` is its twin.
"""

import inspect
from collections.abc import Awaitable
from datetime import timedelta
from typing import TYPE_CHECKING

import pytest
from goodfetch_collie import (
    AsyncCollie,
    ClosedError,
    Collie,
    DeadlineExceededError,
    GoodfetchError,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.handles import AsyncLocator, AsyncPage

RUN_ID = "01ke6p4ym0j8x3v5wq2rtbn7hc"

CLIENT_LOGGER = "goodfetch.client"
"""The logger the engine's stderr lines go to, which a failed play writes."""


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


async def locator_on(way: Way, page: "AsyncPage", selector: str) -> "AsyncLocator":
    """`page`'s locator for `selector`, its methods async whatever the way."""
    built = page.locator(selector)
    if inspect.isawaitable(built):
        # The sync way's proxy builds it in a worker thread, as any member.
        return way.api(built, await built)
    return built


@pytest.mark.covers("story:11-181")
@pytest.mark.client_case(id="raw-call", mode="attached")
@three_ways
async def test_reaches_operations_the_sdk_does_not_generate(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """On the Connection and on each value, under the main path's protections."""
    env = scripted_engine_env(test_env, product, "sdk-python/run/raw-call.json")
    collie = await way.open(
        lambda: AsyncCollie.connect(env=env), lambda: Collie.connect(env=env)
    )

    listed = await collie.call(
        "run.list", {"lifecycle": "running", "tags": ["nightly"]}
    )
    run = await collie.start()
    failed = await run.call("run.failed_requests", {"blocked": True})
    named = await collie.call(
        "run.failed_requests", {"run_id": RUN_ID, "blocked": False}
    )
    other_namespace = await failure(
        run.call("page.goto", {"url": "https://shop.test/"})
    )
    own_param = await failure(run.call("run.usage", {"run_id": "another"}))
    deadline = await failure(run.call("run.finish", timeout=timedelta(milliseconds=2)))
    page = await run.new_page()
    reloaded = await page.call("page.reload")
    clicked = await (await locator_on(way, page, "#consent")).call("locator.click")
    await run.close()
    on_closed_run = await failure(run.call("run.usage"))
    on_closed_page = await failure(page.call("page.reload"))
    await collie.close()
    on_closed_connection = await failure(collie.call("run.list"))

    with client_case.check(
        "the Connection's call answers the engine's result unchanged"
    ):
        assert listed == {
            "runs": [
                {
                    "run_id": RUN_ID,
                    "location": f"/runs/{RUN_ID}",
                    "kind": "crawl",
                    "needs": "foxhound",
                    "tags": ["nightly"],
                    "lifecycle": "running",
                    "started_at": "2026-11-19T12:30:00Z",
                    "presence": "present",
                    "replayed": False,
                }
            ]
        }
    with client_case.check("a value's call fills its own parameter"):
        assert [failed, reloaded, clicked] == [{"requests": []}, {"response": None}, {}]
    with client_case.check("a parameter naming a second value takes its ID"):
        assert named == {"requests": []}
    with client_case.check("an operation of another namespace fails before it is sent"):
        assert (other_namespace.code, other_namespace.group) == (
            "invalid_params",
            "program",
        )
        assert other_namespace.details == {"operation": "page.goto"}
    with client_case.check("the value's own parameter is the value's to fill"):
        assert (own_param.code, own_param.group) == ("invalid_params", "program")
    with client_case.check(
        "the timeout is sent as the operation's own, and its failure raises its class"
    ):
        assert isinstance(deadline, DeadlineExceededError)
    with client_case.check("a call on a closed value fails with closed, unsent"):
        assert [
            type(error)
            for error in (on_closed_run, on_closed_page, on_closed_connection)
        ] == [ClosedError, ClosedError, ClosedError]
    with client_case.check(
        "the script played to its end, so nothing was retried or sent unasked"
    ):
        assert [
            record.getMessage()
            for record in caplog.records
            if record.name == CLIENT_LOGGER
        ] == []
