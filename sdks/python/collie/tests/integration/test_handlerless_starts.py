"""The handler-less starts (ADRs 0036 and 0102), the `handlerless-starts` client case.

A Recipe's run and a code-free catalog run of each target kind, started
with every start option on the Scripted engine, three ways: async on
asyncio, async on Trio and through the sync `Collie`. The engine checks
every message the SDK sends and fails on a step left unused, so a clean
play says each start carried exactly what its script holds. A Run ID
resumes an interrupted Run, logged at info, or returns a finished one
ended, logged at warning. As a Daemon, each Run is detached, so closing its
value and the Connection sends no `run.abort`: the Daemon keeps the Run
once its program is gone. As the Attached engine, closing a Run's value
aborts it. A view of a Run opened by its Run ID closes without stopping it
either. The last test runs on the executable of the same commit, with the
tags past their bounds that no script may send. TypeScript's
`handlerless-starts.test.ts` is its twin.
"""

import logging
from collections.abc import Mapping
from datetime import timedelta

import pytest
from collie_calls import CLIENT_LOGGER, exit_code, failure, open_collie
from goodfetch_collie import (
    AgentTarget,
    ExtractTarget,
    Limits,
    PlanReplayTarget,
    Recording,
    RecordingMode,
    StartedAs,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

START_LOGGER = "goodfetch.client.run"
"""The logger a start that resumed or found its Run ended goes to."""

PRODUCTS = ExtractTarget(
    url="https://shop.example/products",
    schema={"type": "object", "properties": {"productName": {"type": "string"}}},
)
"""The Extraction both scripts start, its instruction aside."""


def start_lines(
    caplog: pytest.LogCaptureFixture, resumed: str, ended: str
) -> tuple[list[tuple[int, str, object, object]], list[tuple[int, str, str, str]]]:
    """What the starts logged, and what resuming `resumed` and ending `ended` log."""
    logged = [
        (
            record.levelno,
            record.getMessage(),
            getattr(record, "run_id", None),
            getattr(record, "started_as", None),
        )
        for record in caplog.records
        if record.name == START_LOGGER
    ]
    expected = [
        (
            logging.INFO,
            f"The start resumed the Run `{resumed}` from its committed state.",
            resumed,
            "resumed",
        ),
        (
            logging.WARNING,
            f"The start found the Run `{ended}` ended: it is returned as it "
            "ended and takes no new work.",
            ended,
            "ended",
        ),
    ]
    return logged, expected


@pytest.mark.covers(
    "story:11-131", "operation:recipe.start", "operation:catalog_run.start"
)
@pytest.mark.client_case(id="handlerless-starts", mode="daemon")
@three_ways
async def test_a_handlerless_start_takes_every_option_and_leaves_its_detached_run(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Each start sends every option given, and closing stops no detached Run."""
    caplog.set_level(logging.INFO, logger=START_LOGGER)
    daemon = start_scripted_daemon(
        test_env, "sdk-python/run/detached-starts-from-a-daemon.json", "socket"
    )
    collie = await open_collie(way, test_env.env, engine="daemon")

    recipe = await collie.start_recipe(
        "shop-prices",
        run_id="nightly-prices",
        tags=["nightly", "prices"],
        limits=Limits(requests=500),
        settings={"timeout": {"wait": timedelta(seconds=5)}},
        recording=Recording(mode=RecordingMode.replay, path="recordings/shop.warc"),
        timeout=timedelta(hours=1),
        detached=True,
    )
    extract = await collie.start_catalog_run(
        ExtractTarget(
            url=PRODUCTS.url,
            schema=PRODUCTS.schema,
            instruction="Read each product's name.",
        ),
        run_id="extract-products",
        detached=True,
    )
    resumed = await collie.start_recipe(
        "shop-prices", run_id="weekly-prices", detached=True
    )
    ended = await collie.start_catalog_run(
        PlanReplayTarget(plan="plans/checkout.json"),
        run_id="checkout-done",
        detached=True,
    )
    agent = await collie.start_catalog_run(
        AgentTarget(
            url="https://shop.example/",
            task="Find the cheapest kettle.",
            expect={"found": True},
            schema={"type": "object", "properties": {"unitPrice": {"type": "number"}}},
            max_steps=20,
        ),
        run_id="cheapest-kettle",
        timeout=None,
        detached=True,
    )
    replay = await collie.start_catalog_run(
        PlanReplayTarget(plan="plans/checkout.json"),
        run_id="checkout-replay",
        recording=Recording(mode=RecordingMode.record, path="recordings/checkout.warc"),
        detached=True,
    )
    await recipe.close()
    view = await collie.open_run("checkout-replay")
    await view.close()
    await collie.close()
    code = await exit_code(daemon.wait)

    with client_case.check("each start answers its Run as the engine started it"):
        assert [
            (run.id, run.started_as)
            for run in (recipe, extract, resumed, ended, agent, replay)
        ] == [
            ("nightly-prices", StartedAs.new),
            ("extract-products", StartedAs.new),
            ("weekly-prices", StartedAs.resumed),
            ("checkout-done", StartedAs.ended),
            ("cheapest-kettle", StartedAs.new),
            ("checkout-replay", StartedAs.new),
        ]
    with client_case.check(
        "the resume is logged at info, the ended Run at warning, on its logger"
    ):
        logged, expected = start_lines(caplog, "weekly-prices", "checkout-done")
        assert logged == expected
    with client_case.check("open_run opens a view by its Run ID with no message"):
        assert view.id == "checkout-replay"
    with client_case.check(
        "the Daemon heard every option and no run.abort, so it played its script whole"
    ):
        assert code == 0


@pytest.mark.covers(
    "story:11-131", "operation:recipe.start", "operation:catalog_run.start"
)
@pytest.mark.client_case(id="handlerless-starts", mode="attached")
@three_ways
async def test_a_handlerless_start_sends_every_option_and_aborts_its_run_on_close(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A start not detached is aborted on close; a resume and an end are logged."""
    caplog.set_level(logging.INFO, logger=START_LOGGER)
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/handlerless-starts.json"
    )
    collie = await open_collie(way, env)

    begun = await collie.start_recipe(
        "shop-prices",
        run_id="nightly-prices",
        tags=["nightly", "prices"],
        limits=Limits(requests=500),
        settings={"timeout": {"wait": timedelta(seconds=5)}},
        recording=Recording(mode=RecordingMode.replay, path="recordings/shop.warc"),
        timeout=timedelta(hours=1),
    )
    await begun.close()
    resumed = await collie.start_recipe("shop-prices", run_id="nightly-prices")
    await resumed.finish()
    ended = await collie.start_catalog_run(PRODUCTS, run_id="extract-products")
    view = await collie.open_run("checkout-replay")
    await view.close()
    await collie.close()

    with client_case.check("each start answers its Run as the engine started it"):
        assert [(run.id, run.started_as) for run in (begun, resumed, ended)] == [
            ("nightly-prices", StartedAs.new),
            ("nightly-prices", StartedAs.resumed),
            ("extract-products", StartedAs.ended),
        ]
    with client_case.check(
        "the resume is logged at info, the ended Run at warning, on its logger"
    ):
        logged, expected = start_lines(caplog, "nightly-prices", "extract-products")
        assert logged == expected
    with client_case.check("open_run opens a view by its Run ID with no message"):
        assert view.id == "checkout-replay"
    with client_case.check(
        "the script played to its end, its Run aborted on close, "
        "so its engine wrote no failure"
    ):
        assert [
            record.getMessage()
            for record in caplog.records
            if record.name == CLIENT_LOGGER
        ] == []


@pytest.mark.covers("story:11-131")
@three_ways
async def test_tags_past_their_bounds_are_refused_on_the_executable(
    way: Way, env: Mapping[str, str]
) -> None:
    """The engine refuses tags past their bounds, and runs no target yet."""
    collie = await open_collie(way, env)
    replay = PlanReplayTarget(plan="plans/checkout.json")

    too_many = await failure(collie.start_recipe("shop-prices", tags=["t"] * 17))
    too_long = await failure(collie.start_catalog_run(replay, tags=["t" * 65]))
    unbuilt = await failure(collie.start_catalog_run(replay, tags=["t" * 64] * 16))
    await collie.close()

    assert [error.code for error in (too_many, too_long, unbuilt)] == [
        "invalid_params",
        "invalid_params",
        "run_target_unbuilt",
    ]
    assert (unbuilt.operation, unbuilt.target) == ("catalog_run.start", "plan_replay")
