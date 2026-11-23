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
either. The last two tests send the tags past their bounds that no script
may send, to an Attached engine and to the Daemon of the executable of the
same commit. TypeScript's `handlerless-starts.test.ts` is its twin.
"""

import logging
from collections.abc import Mapping
from datetime import timedelta

import pytest
from collie_calls import (
    CLIENT_LOGGER,
    START_LOGGER,
    exit_code,
    failure,
    open_collie,
    start_lines,
)
from goodfetch_collie import (
    AgentTarget,
    EngineChoice,
    ExtractTarget,
    GoodfetchError,
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

PRODUCTS = ExtractTarget(
    url="https://shop.example/products",
    schema={"type": "object", "properties": {"productName": {"type": "string"}}},
)
"""The Extraction both scripts start, its instruction aside."""


def resumed_and_ended(resumed: str, ended: str) -> list[tuple[int, str, str, str]]:
    """The lines a resume of `resumed` and an end of `ended` log."""
    return [
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
        assert start_lines(caplog) == resumed_and_ended(
            "weekly-prices", "checkout-done"
        )
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
        assert start_lines(caplog) == resumed_and_ended(
            "nightly-prices", "extract-products"
        )
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


async def refused_tags(
    way: Way, env: Mapping[str, str], engine: EngineChoice
) -> list[GoodfetchError]:
    """What the engine `engine` names answers three starts' tags.

    Too many tags, a tag too long, and tags at their bounds.
    """
    collie = await open_collie(way, env, engine=engine)
    replay = PlanReplayTarget(plan="plans/checkout.json")

    too_many = await failure(collie.start_recipe("shop-prices", tags=["t"] * 17))
    too_long = await failure(collie.start_catalog_run(replay, tags=["t" * 65]))
    unbuilt = await failure(collie.start_catalog_run(replay, tags=["t" * 64] * 16))
    await collie.close()
    return [too_many, too_long, unbuilt]


def assert_refused(refused: list[GoodfetchError]) -> None:
    """Checks the tags past their bounds refused and those at them sent.

    The engine runs no target of the start the tags at their bounds reach.
    """
    too_many, too_long, unbuilt = refused
    assert (too_many.code, too_long.code, unbuilt.code) == (
        "invalid_params",
        "invalid_params",
        "run_target_unbuilt",
    )
    assert (unbuilt.operation, unbuilt.target) == ("catalog_run.start", "plan_replay")


@pytest.mark.covers("story:11-131")
@pytest.mark.client_case(id="handlerless-starts", mode="attached")
@three_ways
async def test_tags_past_their_bounds_are_refused_by_an_attached_engine(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """The executable of the same commit refuses them, and runs no target yet."""
    refused = await refused_tags(way, env, "attached")

    with client_case.check(
        "tags past their bounds are refused, and tags at them reach the start"
    ):
        assert_refused(refused)


@pytest.mark.covers("story:11-131")
@pytest.mark.client_case(id="handlerless-starts", mode="daemon")
@pytest.mark.parametrize("with_daemon", [True])
@three_ways
async def test_tags_past_their_bounds_are_refused_by_the_daemon(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """The Daemon of the same commit refuses them, and runs no target yet."""
    refused = await refused_tags(way, env, "daemon")

    with client_case.check(
        "tags past their bounds are refused, and tags at them reach the start"
    ):
        assert_refused(refused)
