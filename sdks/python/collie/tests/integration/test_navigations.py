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
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct


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


@pytest.mark.covers("story:11-84", "operation:page.goto")
@pytest.mark.client_case(
    id="goto-skips-a-redirects-retry-after-and-answers-none-for-a-file",
    mode="attached",
)
@three_ways
async def test_a_goto_answers_a_followed_redirects_target_and_none_for_a_file(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The Client's side of the case: the answers the engine gives, kept as given.

    The engine follows a 302 whose `Retry-After` asks for an hour and answers
    its target's response, answers no response for a file the browser
    downloads, and a reload answers the document the page kept (ADRs 0086,
    0118, 0119). `test_page_attached.py` runs the same case on the Proving
    ground, where the engine's own part shows. This one plays the corpus's
    `sdk-python/page/redirect-and-file.json`.
    """
    env = scripted_engine_env(
        test_env, product, "sdk-python/page/redirect-and-file.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    followed = await page.goto("https://shop.test/retry-after/302")
    await page.goto("https://shop.test/catalog")
    downloaded = await page.goto("https://shop.test/files/report.pdf")
    reloaded = await page.reload()
    await run.finish()
    await collie.close()

    with client_case.check("a followed redirect answers its target's response"):
        assert followed is not None
        assert (followed.status, followed.url) == (200, "https://shop.test/target")
    with client_case.check("a goto to a file the browser downloads answers None"):
        assert downloaded is None
    with client_case.check("a reload answers the document the page kept"):
        assert reloaded is not None
        assert (reloaded.status, reloaded.url) == (200, "https://shop.test/catalog")
    with client_case.check("the Scripted engine played its script whole"):
        assert engine_warnings(caplog) == []
