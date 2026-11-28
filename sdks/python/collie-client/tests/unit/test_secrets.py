"""Unit tests for `goodfetch_collie_client.secrets` (ADRs 0067 and 0092, issue 1281)."""

from __future__ import annotations

import json
import threading

import pytest
from goodfetch_collie_client._rpc import reply_of_error
from goodfetch_collie_client.generated.errors import SecretUnavailableError
from goodfetch_collie_client.secrets import (
    SecretRequest,
    _handle,
    sync_secret_resolver,
)

CANARY = "gf-canary-secret-value-9f8e7d6c"


@pytest.mark.anyio
async def test_handle_passes_request_and_returns_value() -> None:
    """`secrets.resolve` passes name, run_id and domain and returns the value."""
    seen: list[SecretRequest] = []

    async def resolve(request: SecretRequest) -> str:
        seen.append(request)
        return CANARY

    with_domain = await _handle(
        resolve,
        {"name": "shop_password", "run_id": "nightly", "domain": "shop.test"},
    )
    without_domain = await _handle(
        resolve,
        {"name": "api_key", "run_id": "nightly"},
    )

    assert with_domain == {"value": CANARY}
    assert without_domain == {"value": CANARY}
    assert seen == [
        SecretRequest(name="shop_password", run_id="nightly", domain="shop.test"),
        SecretRequest(name="api_key", run_id="nightly", domain=None),
    ]


@pytest.mark.anyio
async def test_thrown_error_is_redacted_with_no_canary() -> None:
    """A raised exception becomes a redacted secret_unavailable with no canary."""

    async def resolve(_request: SecretRequest) -> str:
        msg = f"vault failed while reading {CANARY}"
        raise RuntimeError(msg)

    with pytest.raises(SecretUnavailableError) as caught:
        await _handle(resolve, {"name": "shop_password", "run_id": "nightly"})

    reply = reply_of_error(caught.value)
    assert reply["data"]["code"] == "secret_unavailable"  # type: ignore[index]
    assert reply["data"]["failure"] == "resolver_failed"  # type: ignore[index]
    assert CANARY not in json.dumps(reply)
    assert CANARY not in str(caught.value)
    assert CANARY not in repr(caught.value)


@pytest.mark.anyio
async def test_thrown_secret_unavailable_keeps_failure_and_strips_canary() -> None:
    """A raised SecretUnavailableError keeps its failure tag and strips its message."""

    async def resolve(_request: SecretRequest) -> str:
        raise SecretUnavailableError(
            f"missing {CANARY}",
            details={"failure": "missing", "leaked": CANARY},
        )

    with pytest.raises(SecretUnavailableError) as caught:
        await _handle(resolve, {"name": "shop_password", "run_id": "nightly"})

    reply = reply_of_error(caught.value)
    assert reply["data"]["code"] == "secret_unavailable"  # type: ignore[index]
    assert reply["data"]["failure"] == "missing"  # type: ignore[index]
    assert CANARY not in json.dumps(reply)


@pytest.mark.anyio
async def test_sync_secret_resolver_runs_on_worker_thread() -> None:
    """The sync wrapper runs the plain resolver on a worker thread."""
    loop_thread = threading.get_ident()
    ran_on: list[int] = []

    def resolve(request: SecretRequest) -> str:
        ran_on.append(threading.get_ident())
        return f"value-for-{request.name}"

    wrapped = sync_secret_resolver(resolve)
    result = await _handle(wrapped, {"name": "tok", "run_id": "nightly"})
    assert result == {"value": "value-for-tok"}
    assert len(ran_on) == 1
    assert ran_on[0] != loop_thread
