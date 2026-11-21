"""Errors and outcomes (ADR 0079), on the Scripted engine as a Collie engine.

Three ways: async on asyncio, async on Trio and through the sync `Collie`.
A navigation to a missing page returns its 404 response, an outcome, where
each failure raises its code's class under its group's, a code no catalog
has raising its group's class with the code kept, and carries its code,
group, retryable flag, operation, call and Run IDs, the fields that apply
and its details. The program fills a field with a Secret's value, the
canary (ADR 0067): no rendering of any error, its string form, `repr` or
traceback, nor anything the SDK logged, holds it. TypeScript's
`errors-and-outcomes.test.ts` is its twin.
"""

import logging
import traceback
from typing import TYPE_CHECKING

import pytest
from collie_calls import engine_warnings, failure, open_collie
from goodfetch_collie import (
    DeadlineExceededError,
    RefusedError,
    UnavailableError,
)
from goodfetch_collie_client.generated.errors import (
    EnginePanickedError,
    InternalError,
    ModelError,
    PageError,
    ProgramError,
    SecretInputRefusedError,
    SecretUnavailableError,
    SecretUnboundError,
    TargetNotUniqueError,
)
from goodfetch_test_support import (
    Canary,
    ClientCaseRecord,
    NamedOutput,
    TestEnv,
    Way,
    scripted_engine_env,
    search_canaries,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.handles import AsyncLocator, AsyncPage

RUN_ID = "01ke6p4ym0j8x3v5wq2rtbn7hc"
"""The Run the script starts."""

CANARY = Canary(
    id="card-typed-password",
    kind="typed_password",
    value="typed-password-canary-3f9b2d71c8e0",
)
"""The Secret's value the program fills `#card` with, which no error may render."""

ACTION_TIMEOUT = ("timeout.action", 5000, "run", "argument")
"""The timeout the script's page failures outlived, set by the call's argument."""


def locator_on(way: Way, page: "AsyncPage", selector: str) -> "AsyncLocator":
    """`page`'s locator for `selector`, its methods async whatever the way.

    A builder is a plain method in both APIs (ADR 0099), so the sync way's
    proxy hands the sync `Locator` itself, which `way.api` wraps.
    """
    built = page.locator(selector)
    return way.api(built, built)


@pytest.mark.covers(
    "story:11-28",
    "story:11-30",
    "story:11-187",
    "error:secret_unbound",
    "error:secret_input_refused",
    "error:target_not_unique",
    "error:deadline_exceeded",
    "error:secret_unavailable",
    "error:engine_panicked",
)
@pytest.mark.client_case(id="errors-and-outcomes", mode="attached")
@three_ways
async def test_returns_an_outcome_and_raises_each_failure_as_its_codes_class(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A 404 returned, then one failure in each group, rendering no Secret."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/errors-and-outcomes.json"
    )
    caplog.set_level(logging.DEBUG, logger="goodfetch")
    collie = await open_collie(way, env)

    run = await collie.start()
    page = await run.new_page()
    missing = await page.goto("https://shop.test/missing")
    unbound = await failure(locator_on(way, page, "#user").fill("%shop_user%"))
    input_refused = await failure(
        locator_on(way, page, "#password").fill("%shop_password%")
    )
    not_unique = await failure(locator_on(way, page, "#buy").click())
    deadline = await failure(locator_on(way, page, "#card").fill(CANARY.value))
    unavailable = await failure(locator_on(way, page, "#otp").fill("%shop_otp%"))
    from_outside = await failure(locator_on(way, page, "#next").click())
    panicked = await failure(locator_on(way, page, "#done").click())
    await run.finish()
    await collie.close()
    errors = [
        unbound,
        input_refused,
        not_unique,
        deadline,
        unavailable,
        from_outside,
        panicked,
    ]

    with client_case.check("a 404 is the navigation's outcome, returned, not raised"):
        assert missing.response is not None
        assert (missing.response.status, missing.response.url) == (
            404,
            "https://shop.test/missing",
        )
    with client_case.check("each failure carries its operation, call ID and Run ID"):
        assert [(error.operation, error.call_id, error.run_id) for error in errors] == [
            (operation, f"call-{index}", RUN_ID)
            for index, operation in enumerate(
                [
                    "locator.fill",
                    "locator.fill",
                    "locator.click",
                    "locator.fill",
                    "locator.fill",
                    "locator.click",
                    "locator.click",
                ],
                start=1,
            )
        ]
    with client_case.check(
        "an unbound placeholder raises SecretUnboundError, a ProgramError naming"
        " the placeholders bound"
    ):
        assert isinstance(unbound, SecretUnboundError)
        assert isinstance(unbound, ProgramError)
        assert (unbound.code, unbound.group, unbound.retryable) == (
            "secret_unbound",
            "program",
            False,
        )
        assert (unbound.target, unbound.url) == ("#user", "https://shop.test/login")
        assert unbound.fix == (
            "Bind `shop_user` to a Secret reference in the Run's `secrets`, or"
            " use a placeholder it binds"
        )
        assert unbound.details == {
            "placeholder": "shop_user",
            "bound": ["shop_password", "shop_otp"],
        }
    with client_case.check(
        "a refused input raises SecretInputRefusedError, a RefusedError naming"
        " the Secret's reference"
    ):
        assert isinstance(input_refused, SecretInputRefusedError)
        assert isinstance(input_refused, RefusedError)
        assert (
            input_refused.code,
            input_refused.group,
            input_refused.retryable,
        ) == ("secret_input_refused", "refused", False)
        assert input_refused.target == "#password"
        assert input_refused.details == {
            "placeholder": "shop_password",
            "reference": "env:SHOP_PASSWORD",
            "domain": "shop.test",
            "reason": "invalid_certificate_exception",
        }
    with client_case.check(
        "a locator matching two elements raises TargetNotUniqueError, a"
        " PageError with its limit and call log"
    ):
        assert isinstance(not_unique, TargetNotUniqueError)
        assert isinstance(not_unique, PageError)
        assert (not_unique.code, not_unique.group, not_unique.retryable) == (
            "target_not_unique",
            "page",
            False,
        )
        assert (not_unique.target, not_unique.url) == (
            "#buy",
            "https://shop.test/cart",
        )
        assert not_unique.limit is not None
        assert (
            not_unique.limit.key,
            not_unique.limit.value,
            not_unique.limit.layer,
            not_unique.limit.origin,
        ) == ACTION_TIMEOUT
        assert not_unique.call_log == ["waiting for #buy", "#buy matched 2 elements"]
        assert not_unique.details == {
            "matches": [
                {
                    "element": '<button id="buy">Buy now</button>',
                    "locator": "getByRole('button', { name: 'Buy now' })",
                },
                {
                    "element": '<button id="buy">Buy later</button>',
                    "locator": "getByRole('button', { name: 'Buy later' })",
                },
            ],
            "elapsed": 5000,
        }
    with client_case.check(
        "an outlived timeout raises DeadlineExceededError, retryable, saying"
        " what it observed and which limit"
    ):
        assert isinstance(deadline, DeadlineExceededError)
        assert (deadline.code, deadline.group, deadline.retryable) == (
            "deadline_exceeded",
            "deadline_exceeded",
            True,
        )
        assert deadline.target == "#card"
        assert deadline.observed == "the field stayed disabled"
        assert deadline.limit is not None
        assert (
            deadline.limit.key,
            deadline.limit.value,
            deadline.limit.layer,
            deadline.limit.origin,
        ) == ACTION_TIMEOUT
        assert deadline.fix == (
            "Wait for #card to be enabled first, or raise the call's timeout"
        )
        assert deadline.details == {"elapsed": 5000, "waited_for": "target_editable"}
    with client_case.check(
        "a Secret with no value raises SecretUnavailableError, an"
        " UnavailableError naming its provider"
    ):
        assert isinstance(unavailable, SecretUnavailableError)
        assert isinstance(unavailable, UnavailableError)
        assert (unavailable.code, unavailable.group, unavailable.retryable) == (
            "secret_unavailable",
            "unavailable",
            False,
        )
        assert unavailable.fix == "Set SHOP_OTP in the engine's environment"
        assert unavailable.details == {
            "placeholder": "shop_otp",
            "reference": "env:SHOP_OTP",
            "provider": "env",
            "failure": "missing",
        }
    with client_case.check(
        "a code no catalog has raises its group's class, the code kept"
    ):
        assert type(from_outside) is ModelError
        assert (
            from_outside.code,
            from_outside.group,
            from_outside.retryable,
            from_outside.target,
        ) == ("model_output_invalid", "model", True, "#next")
        assert from_outside.details == {"attempts": 2}
    with client_case.check(
        "an engine panic raises EnginePanickedError, an InternalError"
    ):
        assert isinstance(panicked, EnginePanickedError)
        assert isinstance(panicked, InternalError)
        assert (panicked.code, panicked.group, panicked.retryable) == (
            "engine_panicked",
            "internal",
            False,
        )
        assert panicked.details == {
            "ended": "call",
            "id": "panic-1",
            "crash_report": "/srv/goodfetch/crashes/panic-1.json",
        }
    with client_case.check(
        "no error's string form, repr or traceback, nor any log record, holds"
        " the Secret"
    ):
        rendered = [
            output
            for index, error in enumerate(errors)
            for output in (
                NamedOutput(f"string {index}", str(error)),
                NamedOutput(f"repr {index}", repr(error)),
                NamedOutput(
                    f"traceback {index}", "".join(traceback.format_exception(error))
                ),
            )
        ]
        logged = [
            NamedOutput(f"log {index}", f"{record.getMessage()} {vars(record)!r}")
            for index, record in enumerate(caplog.records)
        ]
        assert search_canaries([CANARY], [*rendered, *logged]) == []
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []
