"""Complete error details (ADRs 0035, 0079, 0092, 0101, 0106, 0114, 0115 and 0118).

Three ways: async on asyncio, async on Trio and through the sync `Collie`,
attached and as a Daemon. Each failure carries every detail its code
declares: a fill whose one-time code never came raises
`SecretUnavailableError` with `failure: "no_code"`, and
`page_navigated_during_call` raises `PageNavigatedDuringCallError`,
retryable, naming the document the call read and the one that replaced it.
A navigation a site rate-limits raises `BlockedError` with the Block cause's
wire value, `rate_limit`, and `retry_after` the point in time the site asked
for, in UTC; a cause no catalog names is kept as received, with no time. A
Model provider rate-limiting raises `ModelUnavailableError` with `retry_after`
as a `timedelta`; `effect_uncertain` carries `UncertainEffect` with `request`
as `None` on a Collie Run; `artifact_unavailable_offline` carries `pinned_by`
as a typed list of product and plugin pinners; `too_many_redirects` carries
`hops` and `limit`. TypeScript's `complete-error-details.test.ts` is its twin.
"""

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

import pytest
from collie_calls import exit_code, failure, locator_on, open_collie
from goodfetch_collie import (
    ArtifactUnavailableOfflineError,
    AsyncCollie,
    BlockedError,
    EffectUncertainError,
    GoodfetchError,
    ModelUnavailableError,
    PageError,
    PageNavigatedDuringCallError,
    RefusedError,
    SecretUnavailableError,
    TooManyRedirectsError,
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
    "story:11-38",
    "story:11-39",
    "story:11-191",
    "error:secret_unavailable",
    "error:page_navigated_during_call",
    "error:blocked",
    "error:model_unavailable",
    "error:effect_uncertain",
    "error:artifact_unavailable_offline",
    "error:too_many_redirects",
    "block-cause:rate_limit",
)
"""What both modes' tests cover."""


@dataclass(frozen=True, slots=True)
class Played:
    """What the case's calls answered."""

    no_code: GoodfetchError
    """The fill of the one-time code no provider made."""
    navigated: GoodfetchError
    """The click whose document a navigation it did not start replaced."""
    rate_limited: GoodfetchError
    """The navigation the site rate-limited."""
    unknown_cause: GoodfetchError
    """The navigation the site refused with a cause no catalog names."""
    model_unavailable: GoodfetchError
    """The call whose Model provider rate-limited it with a wait duration."""
    effect_uncertain: GoodfetchError
    """The call whose Effect was left uncertain on a Collie Run."""
    artifact_offline: GoodfetchError
    """The call refused by `offline` for a pinned browser artifact."""
    too_many_redirects: GoodfetchError
    """The navigation that exceeded the browser's redirect limit."""


async def played(way: Way, collie: AsyncCollie) -> Played:
    """Plays the case's calls on `collie`, then finishes the Run and closes it."""
    run = await collie.start()
    page = await run.new_page()
    no_code = await failure(locator_on(way, page, "#otp").fill("%shop_totp%"))
    navigated = await failure(locator_on(way, page, "#book").click())
    rate_limited = await failure(page.goto("https://shop.test/deals"))
    unknown_cause = await failure(page.goto("https://shop.test/abroad"))
    model_unavailable = await failure(locator_on(way, page, "#summarize").click())
    effect_uncertain = await failure(locator_on(way, page, "#pay").click())
    artifact_offline = await failure(page.goto("https://shop.test/offline"))
    too_many_redirects = await failure(page.goto("https://shop.test/bounce"))
    await run.finish()
    await collie.close()
    return Played(
        no_code=no_code,
        navigated=navigated,
        rate_limited=rate_limited,
        unknown_cause=unknown_cause,
        model_unavailable=model_unavailable,
        effect_uncertain=effect_uncertain,
        artifact_offline=artifact_offline,
        too_many_redirects=too_many_redirects,
    )


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
    rate_limited = answered.rate_limited
    with client_case.check(
        "a rate-limited navigation raises BlockedError with rate_limit"
        " and retry_after a point in time"
    ):
        assert isinstance(rate_limited, BlockedError)
        assert (rate_limited.code, rate_limited.group, rate_limited.retryable) == (
            "blocked",
            "page",
            False,
        )
        assert rate_limited.url == "https://shop.test/deals"
        assert rate_limited.retry_after == datetime(2026, 11, 21, 8, tzinfo=UTC)
        assert rate_limited.details == {
            "cause": "rate_limit",
            "status": 429,
            "retry_after": "2026-11-21T10:00:00+02:00",
        }
    unknown_cause = answered.unknown_cause
    with client_case.check(
        "a Block cause no catalog names is kept as received, with no retry_after"
    ):
        assert isinstance(unknown_cause, BlockedError)
        assert unknown_cause.url == "https://shop.test/abroad"
        assert unknown_cause.retry_after is None
        assert unknown_cause.details == {
            "cause": "geo_block",
            "status": 451,
            "retry_after": None,
        }
    model_unavailable = answered.model_unavailable
    with client_case.check(
        "a rate-limited Model provider raises ModelUnavailableError"
        " with retry_after a timedelta"
    ):
        assert isinstance(model_unavailable, ModelUnavailableError)
        assert isinstance(model_unavailable, UnavailableError)
        assert (
            model_unavailable.code,
            model_unavailable.group,
            model_unavailable.retryable,
        ) == ("model_unavailable", "unavailable", True)
        assert model_unavailable.retry_after == timedelta(milliseconds=1500)
        assert model_unavailable.typed_details is not None
        assert model_unavailable.typed_details.model_dump() == {
            "reason": "rate_limited",
            "retry_after": 1500,
        }
    effect_uncertain = answered.effect_uncertain
    with client_case.check(
        "an uncertain Effect on a Collie Run raises EffectUncertainError"
        " with request None"
    ):
        assert isinstance(effect_uncertain, EffectUncertainError)
        assert isinstance(effect_uncertain, PageError)
        assert (
            effect_uncertain.code,
            effect_uncertain.group,
            effect_uncertain.retryable,
        ) == ("effect_uncertain", "page", False)
        assert effect_uncertain.typed_details is not None
        assert effect_uncertain.typed_details.request is None
        assert effect_uncertain.typed_details.entry == "effect-1"
        assert effect_uncertain.typed_details.key == "charge-order-42"
        assert (
            effect_uncertain.typed_details.reason
            == "browser closed before confirmation"
        )
        assert effect_uncertain.typed_details.first_command == "locator.click"
        assert effect_uncertain.typed_details.since == datetime(
            2026, 11, 21, 8, tzinfo=UTC
        )
    artifact_offline = answered.artifact_offline
    with client_case.check(
        "an offline artifact refusal raises ArtifactUnavailableOfflineError"
        " with pinned_by as a typed list"
    ):
        assert isinstance(artifact_offline, ArtifactUnavailableOfflineError)
        assert isinstance(artifact_offline, RefusedError)
        assert (
            artifact_offline.code,
            artifact_offline.group,
            artifact_offline.retryable,
        ) == ("artifact_unavailable_offline", "refused", False)
        assert artifact_offline.limit is not None
        assert artifact_offline.limit.model_dump(mode="json", exclude_none=True) == {
            "key": "offline",
            "value": True,
            "layer": "run",
        }
        assert (
            artifact_offline.fix
            == "Run `collie browser install` where the cache is writable."
        )
        assert artifact_offline.typed_details is not None
        assert artifact_offline.typed_details.model_dump(mode="json") == {
            "artifact": {
                "kind": "browser",
                "name": "chrome-for-testing",
                "build": "141.0.7390.54-linux64",
            },
            "pinned_by": [
                {"kind": "product", "name": "collie", "version": "0.0.0"},
                {"kind": "plugin", "name": "demo-plugin", "version": "1.2.0"},
            ],
            "cache": "/var/cache/goodfetch/browsers",
        }
    too_many_redirects = answered.too_many_redirects
    with client_case.check(
        "a redirect loop raises TooManyRedirectsError with hops and the browser limit"
    ):
        assert isinstance(too_many_redirects, TooManyRedirectsError)
        assert isinstance(too_many_redirects, PageError)
        assert (
            too_many_redirects.code,
            too_many_redirects.group,
            too_many_redirects.retryable,
        ) == ("too_many_redirects", "page", False)
        assert too_many_redirects.url == "https://shop.test/bounce"
        assert too_many_redirects.typed_details is not None
        assert too_many_redirects.typed_details.model_dump() == {
            "hops": ["https://shop.test/bounce/1", "https://shop.test/bounce/2"],
            "limit": {"kind": "browser", "count": 20},
        }


@COVERS
@pytest.mark.client_case(id="complete-error-details", mode="attached")
@three_ways
async def test_reads_every_detail_each_failures_code_declares(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
) -> None:
    """All complete-error-details failures across the suite.

    Covers `no_code`, `page_navigated_during_call`, `blocked`,
    `model_unavailable`, `effect_uncertain`, `artifact_unavailable_offline`,
    and `too_many_redirects`.
    """
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
