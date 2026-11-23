"""A member whose operation answers one field returns that field's value (issue 2883).

A navigation answers the document's response, or `None` when it has none,
and an evaluation the function's value, on the Scripted engine started as
the executable, three ways: async on asyncio, async on Trio and through the
sync `Collie`. It plays the corpus's `page/navigations.json` with the Python
SDK's hello. TypeScript's `page.test.ts` and Rust's `navigations.rs` are its
twins.
"""

import pytest
from collie_calls import engine_warnings, open_collie
from goodfetch_test_support import Product as ToolProduct
from goodfetch_test_support import TestEnv, Way, scripted_engine_env, three_ways


@pytest.mark.covers("story:11-84")
@three_ways
async def test_a_navigation_answers_its_response_or_none_and_an_evaluation_its_value(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A 404 is a response, a reload with none `None`, `undefined` `None`."""
    env = scripted_engine_env(test_env, product, "sdk-python/page/navigations.json")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    missing = await page.goto("https://shop.test/missing")
    reloaded = await page.reload()
    answered = await page.evaluate("6 * 7")
    left_out = await page.evaluate("undefined")
    await page.close()
    await run.finish()
    await collie.close()

    assert missing is not None, "a 404 is a response"
    assert (missing.status, missing.url, reloaded, answered, left_out) == (
        404,
        "https://shop.test/missing",
        None,
        42,
        None,
    )
    assert engine_warnings(caplog) == [], "the Scripted engine played its script whole"
