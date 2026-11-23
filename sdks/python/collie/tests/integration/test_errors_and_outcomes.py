"""Errors and outcomes (ADR 0079), on the Scripted engine as a Collie engine.

Three ways: async on asyncio, async on Trio and through the sync `Collie`.
A navigation to a missing page returns its 404 response, an outcome, where
each failure raises its code's class under its group's, a code no catalog
has raising its group's class with the code kept, and carries its code,
group, retryable flag, operation, call and Run IDs, the fields that apply
and its details. The program fills a field with a Secret's value, the
canary (ADR 0067): no rendering of any error, its string form, `repr` or
traceback, nor anything the SDK logged, holds it. Played attached, and as a
Daemon, whose script exits 0 only once every step played. TypeScript's
`errors-and-outcomes.test.ts` is its twin.
"""

import logging
import traceback
from dataclasses import dataclass

import pytest
from collie_calls import engine_warnings, exit_code, failure, locator_on, open_collie
from goodfetch_collie import (
    AsyncCollie,
    DeadlineExceededError,
    EnginePanickedError,
    GoodfetchError,
    InternalError,
    ModelError,
    PageError,
    ProgramError,
    RefusedError,
    SecretInputRefusedError,
    SecretUnavailableError,
    SecretUnboundError,
    TargetNotUniqueError,
    UnavailableError,
)
from goodfetch_collie_client.response import AsyncResponse
from goodfetch_test_support import (
    Canary,
    ClientCaseRecord,
    NamedOutput,
    TestEnv,
    Way,
    scripted_engine_env,
    search_canaries,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

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

COVERS = pytest.mark.covers(
    "story:11-28",
    "story:11-30",
    "story:11-187",
    "error:secret_input_refused",
    "error:target_not_unique",
    "error:deadline_exceeded",
    "error:secret_unavailable",
    "error:engine_panicked",
)
"""What both modes' tests cover."""


@dataclass(frozen=True, slots=True)
class Played:
    """What the case's calls answered, in the script's order."""

    missing: AsyncResponse | None
    unbound: GoodfetchError
    input_refused: GoodfetchError
    not_unique: GoodfetchError
    deadline: GoodfetchError
    unavailable: GoodfetchError
    from_outside: GoodfetchError
    panicked: GoodfetchError

    def errors(self) -> list[GoodfetchError]:
        """Each failure, in the script's order."""
        return [
            self.unbound,
            self.input_refused,
            self.not_unique,
            self.deadline,
            self.unavailable,
            self.from_outside,
            self.panicked,
        ]


async def played(way: Way, collie: AsyncCollie) -> Played:
    """Plays the case's calls on `collie`, then finishes the Run and closes it."""
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
    return Played(
        missing=missing,
        unbound=unbound,
        input_refused=input_refused,
        not_unique=not_unique,
        deadline=deadline,
        unavailable=unavailable,
        from_outside=from_outside,
        panicked=panicked,
    )


def check_played(
    client_case: ClientCaseRecord,
    answered: Played,
    records: list[logging.LogRecord],
) -> None:
    """The 404 returned, each failure's class, code, group and details, no Secret.

    No rendering of an error, nor any of `records`, holds the canary.
    """
    errors = answered.errors()
    missing = answered.missing
    unbound = answered.unbound
    input_refused = answered.input_refused
    not_unique = answered.not_unique
    deadline = answered.deadline
    unavailable = answered.unavailable
    from_outside = answered.from_outside
    panicked = answered.panicked
    with client_case.check("a 404 is the navigation's outcome, returned, not raised"):
        assert missing is not None
        assert (missing.status, missing.url) == (
            404,
            "https://shop.test/missing",
        )
    with client_case.check("each failure carries its operation, call ID and Run ID"):
        assert [(error.operation, error.call_id, error.run_id) for error in errors] == [
            ("locator.fill", "call-1", RUN_ID),
            ("locator.fill", "call-2", RUN_ID),
            ("locator.click", "call-3", RUN_ID),
            ("locator.fill", "call-4", RUN_ID),
            ("locator.fill", "call-5", RUN_ID),
            ("locator.click", "call-6", RUN_ID),
            ("locator.click", "call-7", RUN_ID),
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
            for index, record in enumerate(records)
        ]
        assert search_canaries([CANARY], [*rendered, *logged]) == []


@COVERS
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
    answered = await played(way, await open_collie(way, env))

    check_played(client_case, answered, caplog.records)
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@COVERS
@pytest.mark.client_case(id="errors-and-outcomes", mode="daemon")
@three_ways
async def test_returns_the_same_outcome_and_failures_from_a_daemon(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The same 404 and failures from a Daemon, rendering no Secret."""
    daemon = start_scripted_daemon(
        test_env, "sdk-python/run/errors-and-outcomes-from-a-daemon.json", "socket"
    )
    caplog.set_level(logging.DEBUG, logger="goodfetch")
    collie = await open_collie(way, test_env.env, engine="daemon")
    answered = await played(way, collie)
    code = await exit_code(daemon.wait)

    check_played(client_case, answered, caplog.records)
    with client_case.check(
        "the Daemon played its script through, so the Client sent exactly what it holds"
    ):
        assert code == 0
