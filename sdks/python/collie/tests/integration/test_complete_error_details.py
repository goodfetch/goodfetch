"""Complete error details (ADRs 0092 and 0114), on the Scripted engine.

Three ways: async on asyncio, async on Trio and through the sync `Collie`,
attached and as a Daemon. Each failure carries every detail its code
declares: a fill whose one-time code never came raises
`SecretUnavailableError` with `failure: "no_code"`, and
`page_navigated_during_call` raises `PageNavigatedDuringCallError`,
retryable, naming the document the call read and the one that replaced it.
TypeScript's `complete-error-details.test.ts` is its twin.
"""

from dataclasses import dataclass

import pytest
from collie_calls import exit_code, failure, locator_on, open_collie
from goodfetch_collie import (
    AsyncCollie,
    GoodfetchError,
    PageError,
    PageNavigatedDuringCallError,
    SecretUnavailableError,
    UnavailableError,
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

COVERS = pytest.mark.covers(
    "error:secret_unavailable", "error:page_navigated_during_call"
)
"""What both modes' tests cover."""


@dataclass(frozen=True, slots=True)
class Played:
    """What the case's calls answered."""

    no_code: GoodfetchError
    """The fill of the one-time code no provider made."""
    navigated: GoodfetchError
    """The click whose document a navigation it did not start replaced."""


async def played(way: Way, collie: AsyncCollie) -> Played:
    """Plays the case's calls on `collie`, then finishes the Run and closes it."""
    run = await collie.start()
    page = await run.new_page()
    no_code = await failure(locator_on(way, page, "#otp").fill("%shop_totp%"))
    navigated = await failure(locator_on(way, page, "#book").click())
    await run.finish()
    await collie.close()
    return Played(no_code=no_code, navigated=navigated)


def check_played(client_case: ClientCaseRecord, answered: Played) -> None:
    """The case's checks, the same in both modes."""
    no_code = answered.no_code
    with client_case.check(
        "a one-time code that never came raises SecretUnavailableError saying no_code"
    ):
        assert isinstance(no_code, SecretUnavailableError)
        assert isinstance(no_code, UnavailableError)
        assert (no_code.code, no_code.group, no_code.retryable) == (
            "secret_unavailable",
            "unavailable",
            False,
        )
        assert no_code.fix == "Check the TOTP seed of the vault's shop item"
        assert no_code.details == {
            "placeholder": "shop_totp",
            "reference": "bw:shop/totp",
            "provider": "bw",
            "failure": "no_code",
        }
    navigated = answered.navigated
    with client_case.check(
        "a navigation the call did not start raises PageNavigatedDuringCallError"
        " naming both documents"
    ):
        assert isinstance(navigated, PageNavigatedDuringCallError)
        assert isinstance(navigated, PageError)
        assert (navigated.code, navigated.group, navigated.retryable) == (
            "page_navigated_during_call",
            "page",
            True,
        )
        assert navigated.url == "https://travel.test/books"
        assert (
            navigated.fix
            == "Await the Collie call before your driver acts on the page."
        )
        assert navigated.details == {"navigated_to": "https://travel.test/books/1"}


@COVERS
@pytest.mark.client_case(id="complete-error-details", mode="attached")
@three_ways
async def test_reads_every_detail_each_failures_code_declares(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
) -> None:
    """`no_code` and both documents of a navigation the call did not start."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/complete-error-details.json"
    )
    answered = await played(way, await open_collie(way, env))

    check_played(client_case, answered)


@COVERS
@pytest.mark.client_case(id="complete-error-details", mode="daemon")
@three_ways
async def test_reads_every_detail_each_failures_code_declares_from_a_daemon(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
) -> None:
    """The same details from a Daemon, which played its script through."""
    daemon = start_scripted_daemon(
        test_env, "sdk-python/run/complete-error-details-from-a-daemon.json", "socket"
    )
    collie = await open_collie(way, test_env.env, engine="daemon")
    answered = await played(way, collie)
    code = await exit_code(daemon.wait)

    check_played(client_case, answered)
    with client_case.check(
        "the Daemon played its script through, so the Client sent exactly what it holds"
    ):
        assert code == 0
