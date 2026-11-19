"""Results keep what the SDK does not know (ADR 0082, G3).

A newer engine answers a start and a finish with a field this SDK does not
know: the Run report the program receives keeps it in `model_extra`, built
without validation, so drift never raises. The case plays on the Scripted
engine started as the executable, which checks every message the SDK sends,
three ways: async on asyncio, async on Trio and through the sync `Collie`.
TypeScript's `typed-data.test.ts` is its twin; Python's typed values are
proved at the codec's seam (`goodfetch_collie_client`'s `test_schema.py`)
until issue 1284 gives Python its Key-value round trip.
"""

import pytest
from collie_calls import engine_warnings, open_collie
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct


@pytest.mark.covers("story:11-68")
@pytest.mark.client_case(id="result-keeps-unknown-fields", mode="attached")
@three_ways
async def test_keeps_a_field_the_sdk_does_not_know_in_the_results_it_hands_over(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The Run report keeps the newer engine's `carbon` as sent."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/result-keeps-unknown-fields.json"
    )
    collie = await open_collie(way, env)

    run = await collie.start(run_id="newer")
    report = await run.finish()
    await collie.close()

    with client_case.check("the Run report keeps the newer engine's field as sent"):
        assert (report.run_id.root, report.model_extra) == (
            "newer",
            {"carbon": {"grams": 1.5}},
        )
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []
