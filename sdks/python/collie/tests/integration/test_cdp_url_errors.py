"""A `cdp_url` the engine cannot use, one remedy per code (ADR 0115).

On the Scripted engine, three ways: async on asyncio, async on Trio and
through the sync `Collie`. Connecting and starting the Run succeed, and each
opening of the default Session raises the class of its remedy, under its
group, with its `retryable` flag, `fix` and details, naming the address and
the settings layer that gave it.
TypeScript's `cdp-url-errors.test.ts` is its twin.
"""

import pytest
from collie_calls import engine_warnings, failure, open_collie
from goodfetch_collie_client.generated.errors import (
    CDPURLNotAllowedError,
    CDPURLNotBrowserError,
    CDPURLUnauthorizedError,
    CDPURLUnreachableError,
    ProgramError,
    RefusedError,
    UnavailableError,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct


@pytest.mark.covers(
    "story:11-294",
    "error:cdp_url_unreachable",
    "error:cdp_url_not_browser",
    "error:cdp_url_unauthorized",
    "error:cdp_url_not_allowed",
)
@pytest.mark.client_case(id="cdp-url-errors", mode="attached")
@three_ways
async def test_raises_each_cdp_url_codes_class_when_the_session_opens(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Connects and starts its Run; each opening of the Session fails."""
    env = scripted_engine_env(test_env, product, "sdk-python/run/cdp-url-errors.json")
    collie = await open_collie(way, env)
    run = await collie.start()
    unreachable = await failure(run.new_page())
    not_browser = await failure(run.new_page())
    unauthorized = await failure(run.new_page())
    not_allowed = await failure(run.new_page())
    await run.finish()
    await collie.close()

    with client_case.check(
        "a refused connection raises CDPURLUnreachableError, a retryable"
        " UnavailableError naming its failure"
    ):
        assert isinstance(unreachable, CDPURLUnreachableError)
        assert isinstance(unreachable, UnavailableError)
        assert (unreachable.code, unreachable.group, unreachable.retryable) == (
            "cdp_url_unreachable",
            "unavailable",
            True,
        )
        assert unreachable.url == "http://127.0.0.1:9222"
        assert unreachable.fix == (
            "Start the browser with remote debugging on that port, or correct"
            " the address"
        )
        assert unreachable.details == {
            "layer": "user_configuration",
            "failure": "connection_refused",
        }
    with client_case.check(
        "a page target's address raises CDPURLNotBrowserError, a ProgramError"
        " naming what answered and the browser's address"
    ):
        assert isinstance(not_browser, CDPURLNotBrowserError)
        assert isinstance(not_browser, ProgramError)
        assert (not_browser.code, not_browser.group, not_browser.retryable) == (
            "cdp_url_not_browser",
            "program",
            False,
        )
        assert not_browser.url == "ws://127.0.0.1:9222/devtools/page/8C5F0D1E"
        assert not_browser.observed == "a CDP target of type `page`"
        assert not_browser.fix == (
            "Give the browser's endpoint,"
            " ws://127.0.0.1:9222/devtools/browser/1A2B3C4D, which this host's"
            " /json/version lists"
        )
        assert not_browser.details == {
            "layer": "user_configuration",
            "browser_url": "ws://127.0.0.1:9222/devtools/browser/1A2B3C4D",
        }
    with client_case.check(
        "a 401 raises CDPURLUnauthorizedError, a RefusedError naming the header"
        " and its Secret reference"
    ):
        assert isinstance(unauthorized, CDPURLUnauthorizedError)
        assert isinstance(unauthorized, RefusedError)
        assert (unauthorized.code, unauthorized.group, unauthorized.retryable) == (
            "cdp_url_unauthorized",
            "refused",
            False,
        )
        assert unauthorized.url == "https://browser.example.com/"
        assert unauthorized.fix == "Check the Secret references in `headers`"
        assert unauthorized.details == {
            "layer": "user_configuration",
            "status": 401,
            "headers": {"Authorization": "env:BROWSER_TOKEN"},
        }
    with client_case.check(
        "a loopback address from a model's project configuration raises"
        " CDPURLNotAllowedError, a RefusedError naming the rule and layer"
    ):
        assert isinstance(not_allowed, CDPURLNotAllowedError)
        assert isinstance(not_allowed, RefusedError)
        assert (not_allowed.code, not_allowed.group, not_allowed.retryable) == (
            "cdp_url_not_allowed",
            "refused",
            False,
        )
        assert not_allowed.url == "http://127.0.0.1:9222"
        assert not_allowed.fix == (
            "Set the address in Operator policy or in user configuration, or"
            " give a public address"
        )
        assert not_allowed.details == {
            "layer": "project_configuration",
            "rule": "loopback",
            "address": "127.0.0.1",
        }
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []
