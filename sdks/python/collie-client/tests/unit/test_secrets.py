"""Unit tests for `goodfetch_collie_client.secrets` (ADRs 0067 and 0092, issue 1281)."""

from __future__ import annotations

import json
import threading
from typing import TYPE_CHECKING, cast

import anyio
import pytest
from goodfetch_collie_client._framing import Request
from goodfetch_collie_client._rpc import ServedRequest
from goodfetch_collie_client.generated.errors import SecretUnavailableError
from goodfetch_collie_client.secrets import (
    SECRETS_RESOLVE,
    AsyncSecretResolver,
    SecretRequest,
    serve_secret_resolver,
    sync_secret_resolver,
)

if TYPE_CHECKING:
    from collections.abc import Mapping

CANARY = "gf-canary-secret-value-9f8e7d6c"


async def _serve_once(
    resolve: AsyncSecretResolver, params: object
) -> Mapping[str, object]:
    """Dispatches one `secrets.resolve` request through `serve_secret_resolver`."""
    answered = anyio.Event()
    replies: list[Mapping[str, object]] = []

    async def answer(_req: ServedRequest, body: Mapping[str, object]) -> None:
        replies.append(body)
        answered.set()

    served = ServedRequest(Request(id=1, method=SECRETS_RESOLVE, params=params), answer)
    serve_secret_resolver(resolve)(served)
    await answered.wait()
    assert len(replies) == 1
    return replies[0]


@pytest.mark.anyio
async def test_serve_passes_request_and_returns_value() -> None:
    """`secrets.resolve` passes name, run_id and domain and returns the value."""
    seen: list[SecretRequest] = []

    async def resolve(request: SecretRequest) -> str:
        seen.append(request)
        return CANARY

    with_domain = await _serve_once(
        resolve,
        {"name": "shop_password", "run_id": "nightly", "domain": "shop.test"},
    )
    without_domain = await _serve_once(
        resolve,
        {"name": "api_key", "run_id": "nightly"},
    )

    assert with_domain == {"result": {"value": CANARY}}
    assert without_domain == {"result": {"value": CANARY}}
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

    reply = await _serve_once(resolve, {"name": "shop_password", "run_id": "nightly"})
    error = cast("dict[str, object]", reply["error"])
    data = cast("dict[str, object]", error["data"])
    assert data["code"] == "secret_unavailable"
    assert data["failure"] == "resolver_failed"
    assert CANARY not in json.dumps(reply)


@pytest.mark.anyio
async def test_thrown_secret_unavailable_keeps_known_failure_and_strips_canary() -> (
    None
):
    """A raised SecretUnavailableError keeps a known failure tag and strips canaries."""

    async def resolve(_request: SecretRequest) -> str:
        raise SecretUnavailableError(
            f"missing {CANARY}",
            details={"failure": "missing", "leaked": CANARY},
        )

    reply = await _serve_once(resolve, {"name": "shop_password", "run_id": "nightly"})
    error = cast("dict[str, object]", reply["error"])
    data = cast("dict[str, object]", error["data"])
    assert data["code"] == "secret_unavailable"
    assert data["failure"] == "missing"
    assert CANARY not in json.dumps(reply)


@pytest.mark.anyio
async def test_thrown_secret_unavailable_rejects_arbitrary_failure_tag() -> None:
    """An unknown or canary failure string is replaced with resolver_failed."""

    async def resolve(_request: SecretRequest) -> str:
        raise SecretUnavailableError(
            "failed",
            details={"failure": f"leaked-{CANARY}"},
        )

    reply = await _serve_once(resolve, {"name": "shop_password", "run_id": "nightly"})
    error = cast("dict[str, object]", reply["error"])
    data = cast("dict[str, object]", error["data"])
    assert data["code"] == "secret_unavailable"
    assert data["failure"] == "resolver_failed"
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
    reply = await _serve_once(wrapped, {"name": "tok", "run_id": "nightly"})
    assert reply == {"result": {"value": "value-for-tok"}}
    assert len(ran_on) == 1
    assert ran_on[0] != loop_thread
