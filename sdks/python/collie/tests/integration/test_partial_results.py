"""Partial results (ADR 0079), on the Scripted engine as a Collie engine.

Three ways: async on asyncio, async on Trio and through the sync `Collie`.
A value of a declared type is complete: a navigation into a redirect loop
raises `TooManyRedirectsError`, whose details hold what the navigation
gathered before it failed, and a read of several fields answers each
field's own outcome, a value or `None` with why, never one error for the
call. TypeScript's `partial-results.test.ts` is its twin.
"""

import pytest
from collie_calls import failure, open_collie
from goodfetch_collie_client.generated.errors import PageError, TooManyRedirectsError
from goodfetch_collie_client.generated.models import PageField
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

HOPS = [f"https://shop.test/loop/{hop}" for hop in range(1, 20)]
"""The hops the looping navigation followed before the browser's count stopped it."""

FIELDS = {
    "title": PageField.model_validate(
        {"locator": {"kind": "css", "selector": "h1"}, "transform": {"kind": "text"}}
    ),
    "price": PageField.model_validate(
        {
            "locator": {"kind": "css", "selector": ".price"},
            "transform": {"kind": "number", "locale": "en"},
        }
    ),
    "rating": PageField.model_validate(
        {
            "locator": {"kind": "css", "selector": ".rating"},
            "transform": {"kind": "number", "locale": "en"},
        }
    ),
}
"""The three fields the script's read declares."""


@pytest.mark.covers("story:11-35")
@pytest.mark.client_case(id="partial-results", mode="attached")
@three_ways
async def test_a_failure_carries_what_it_gathered_and_a_read_answers_each_field(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
) -> None:
    """A redirect loop's hops on its error's details, then one outcome per field."""
    env = scripted_engine_env(test_env, product, "sdk-python/run/partial-results.json")
    collie = await open_collie(way, env)

    run = await collie.start()
    page = await run.new_page()
    looping = await failure(page.goto("https://shop.test/loop"))
    read = await page.read(FIELDS)
    await run.finish()
    await collie.close()

    with client_case.check(
        "a failed navigation's error holds the hops it followed before failing"
    ):
        assert isinstance(looping, TooManyRedirectsError)
        assert isinstance(looping, PageError)
        assert (looping.code, looping.operation, looping.url) == (
            "too_many_redirects",
            "page.goto",
            "https://shop.test/loop",
        )
        assert looping.details == {
            "hops": HOPS,
            "limit": {"kind": "browser", "count": 20},
        }
    with client_case.check("a read answers each field's own outcome, None with why"):
        assert read.values == {"title": "Blue kettle", "price": None, "rating": None}
        assert [
            (unread.field, unread.reason, unread.text) for unread in read.unread or []
        ] == [
            ("/price", "no_element", None),
            ("/rating", "not_parsed", "five stars"),
        ]
