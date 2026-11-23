"""The handler-less starts (ADRs 0036 and 0102), on the Scripted engine as a Daemon.

A Recipe's run and a code-free catalog run of each target kind, started
with every start option, three ways: async on asyncio, async on Trio and
through the sync `Collie`. The Daemon checks every message the SDK sends
and fails on a step left unused, so its clean exit says each start carried
exactly what its script holds. Each Run is detached, so closing its value
and the Connection sends no `run.abort`: the Daemon keeps the Run once its
program is gone. A view of a Run opened by its Run ID closes without
stopping it either. TypeScript's `handlerless-starts.test.ts` is its twin.
"""

from datetime import timedelta

import pytest
from collie_calls import exit_code, open_collie
from goodfetch_collie import (
    AgentTarget,
    ExtractTarget,
    Limits,
    PlanReplayTarget,
    Recording,
    RecordingMode,
    StartedAs,
)
from goodfetch_test_support import TestEnv, Way, start_scripted_daemon, three_ways


@pytest.mark.covers("operation:recipe.start", "operation:catalog_run.start")
@three_ways
async def test_a_handlerless_start_takes_every_option_and_leaves_its_detached_run(
    way: Way, test_env: TestEnv
) -> None:
    """Each start sends every option given, and closing stops no detached Run."""
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
            url="https://shop.example/products",
            schema={
                "type": "object",
                "properties": {"productName": {"type": "string"}},
            },
            instruction="Read each product's name.",
        ),
        run_id="extract-products",
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

    assert [(run.id, run.started_as) for run in (recipe, extract, agent, replay)] == [
        ("nightly-prices", StartedAs.new),
        ("extract-products", StartedAs.new),
        ("cheapest-kettle", StartedAs.new),
        ("checkout-replay", StartedAs.new),
    ]
    assert view.id == "checkout-replay"
    assert code == 0
