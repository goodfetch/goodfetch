"""The caller as a Secret provider (ADRs 0067, 0092 and 0104; issue 1281).

Played three ways (asyncio, Trio and sync `Collie`), attached and as a Daemon,
against the Scripted engine: `connect(resolve_secret=...)` answers
`secrets.resolve` for `caller:<name>` references at the moment of use, and a
raised exception becomes a redacted `secret_unavailable` with no Secret canary
in any `repr`, `str`, error message, call log or JSON-RPC error.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

import pytest
from collie_calls import exit_code, failure, locator_on, open_collie
from goodfetch_collie import (
    AsyncCollie,
    GoodfetchError,
    SecretRequest,
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

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie import EngineChoice

CANARY = "canary-caller-secret-value-not-real"
COVERS = pytest.mark.covers("story:11-98")


@dataclass(frozen=True, slots=True)
class Played:
    """What the case's calls answered."""

    seen: list[SecretRequest]
    failed: GoodfetchError


async def played(
    way: Way,
    env: Mapping[str, str],
    *,
    engine: EngineChoice = "auto",
) -> Played:
    """Plays `caller-secret-resolution` through `way`."""
    seen: list[SecretRequest] = []

    def sync_resolve(request: SecretRequest) -> str:
        seen.append(request)
        if request.name == "shop_pw":
            return CANARY
        msg = f"vault read failed for {CANARY}"
        raise RuntimeError(msg)

    async def async_resolve(request: SecretRequest) -> str:
        return sync_resolve(request)

    collie: AsyncCollie = await open_collie(
        way,
        env,
        engine=engine,
        async_resolve_secret=async_resolve,
        sync_resolve_secret=sync_resolve,
    )
    run = await collie.start()
    page = await run.new_page()
    await locator_on(way, page, "#pw").fill("%shop_pw%")
    failed = await failure(locator_on(way, page, "#otp").fill("%shop_otp%"))
    await run.finish()
    await collie.close()
    return Played(seen=seen, failed=failed)


def check_played(
    client_case: ClientCaseRecord,
    answered: Played,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The case's checks, the same in both modes."""
    with client_case.check(
        "resolve_secret is called once per caller: reference"
        " with name, run_id and domain"
    ):
        assert answered.seen == [
            SecretRequest(
                name="shop_pw",
                run_id="01ke6p4ym0j8x3v5wq2rtbn7hc",
                domain="shop.test",
            ),
            SecretRequest(
                name="shop_otp",
                run_id="01ke6p4ym0j8x3v5wq2rtbn7hc",
                domain="shop.test",
            ),
        ]
    with client_case.check(
        "a throw becomes SecretUnavailableError with failure resolver_failed"
        " and no canary in repr, str, call_log or logs"
    ):
        failed = answered.failed
        assert isinstance(failed, SecretUnavailableError)
        assert isinstance(failed, UnavailableError)
        assert (failed.code, failed.group, failed.retryable) == (
            "secret_unavailable",
            "unavailable",
            False,
        )
        assert failed.details["placeholder"] == "shop_otp"
        assert failed.details["reference"] == "caller:shop_otp"
        assert failed.details["provider"] == "caller"
        assert failed.details["failure"] == "resolver_failed"
        assert failed.fix is not None
        assert "resolve_secret" in failed.fix
        assert CANARY not in str(failed)
        assert CANARY not in repr(failed)
        assert CANARY not in repr(failed.call_log)
        assert CANARY not in caplog.text


@COVERS
@pytest.mark.client_case(id="caller-secret-resolution", mode="attached")
@three_ways
async def test_answers_caller_references_and_redacts_failures(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Attached: `resolve_secret` answers `caller:` and redacts exceptions."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/caller-secret-resolution.json"
    )
    answered = await played(way, env)
    check_played(client_case, answered, caplog)


@COVERS
@pytest.mark.client_case(id="caller-secret-resolution", mode="daemon")
@three_ways
async def test_answers_caller_references_from_a_daemon(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Daemon: `resolve_secret` answers `caller:` and plays its script through."""
    daemon = start_scripted_daemon(
        test_env,
        "sdk-python/run/caller-secret-resolution-from-a-daemon.json",
        "socket",
    )
    answered = await played(way, test_env.env, engine="daemon")
    code = await exit_code(daemon.wait)

    check_played(client_case, answered, caplog)
    with client_case.check(
        "the Daemon played its script through, so no canary reached the JSON-RPC error"
    ):
        assert code == 0
