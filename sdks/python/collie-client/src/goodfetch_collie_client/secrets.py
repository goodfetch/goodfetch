"""The caller as a Secret provider (ADRs 0067, 0092 and 0104).

`connect(resolve_secret=...)` answers `caller:<name>` Secret references at the
moment of use, for Runs its own Connection opened. The value goes straight to
the engine; nothing here keeps a copy, logs it or repeats a raised exception's
message in the JSON-RPC reply, since a raised exception may hold a Secret value.
TypeScript's `secrets.ts` is its twin.
"""

from __future__ import annotations

import functools
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING, Final, TypeAlias, cast

import anyio.to_thread

from goodfetch_collie_client.generated.errors import SecretUnavailableError
from goodfetch_collie_client.listeners import spawner

if TYPE_CHECKING:
    from goodfetch_collie_client._rpc import ServedRequest

__all__ = [
    "AsyncSecretResolver",
    "SecretRequest",
    "SecretResolver",
]

SECRETS_RESOLVE = "secrets.resolve"
"""The handler request the engine sends for a `caller:<name>` Secret reference."""


@dataclass(frozen=True)
class SecretRequest:
    """What the engine passes to `resolve_secret` for a `caller:<name>` reference.

    ADR 0092. Its `repr` holds only the reference's name, Run ID and domain,
    never a Secret value.
    """

    name: str
    """The reference's name, what follows `caller:`."""
    run_id: str
    """The Run that uses it, which this Connection opened."""
    domain: str | None = None
    """The host the value is sent to, or `None` where the use has none."""


AsyncSecretResolver: TypeAlias = Callable[[SecretRequest], Awaitable[str]]
"""Answers `caller:<name>` Secret references on the async client (ADR 0092)."""

SecretResolver: TypeAlias = Callable[[SecretRequest], str]
"""Answers `caller:<name>` Secret references on the sync client (ADR 0092)."""


def sync_secret_resolver(resolve_secret: SecretResolver) -> AsyncSecretResolver:
    """Runs a plain sync `resolve_secret` on the Connection's thread pool."""

    async def _resolve(request: SecretRequest) -> str:
        return await anyio.to_thread.run_sync(resolve_secret, request)

    return _resolve


def serve_secret_resolver(
    resolve_secret: AsyncSecretResolver,
) -> Callable[[ServedRequest], None]:
    """Builds the `secrets.resolve` server for `ClientConnection.serve`.

    Starts each resolution on the Connection's event loop via `spawner()` so
    the reading task never blocks, returns `{ "value": value }` straight
    through without keeping or logging it, and turns any exception or
    non-string return into a redacted `SecretUnavailableError` with no text
    from the raised exception (ADR 0067).
    """

    def _served(request: ServedRequest) -> None:
        spawner()(
            functools.partial(
                request.serve,
                functools.partial(_handle, resolve_secret, request.params),
            )
        )

    return _served


_SECRET_FAILURES: Final[frozenset[str]] = frozenset(
    {
        "missing",
        "not_configured",
        "provider_failed",
        "resolver_failed",
        "timed_out",
        "no_code",
    }
)
"""Closed set of wire `SecretFailure` tags (ADR 0067)."""


async def _handle(
    resolve_secret: AsyncSecretResolver, params: object
) -> dict[str, str]:
    request = _parse_secret_request(params)
    try:
        value = cast("object", await resolve_secret(request))
    except Exception as error:
        raise _redacted_unavailable(error) from None
    if not isinstance(value, str):
        raise SecretUnavailableError(
            "resolve_secret returned no string value",
            details={"failure": "resolver_failed"},
        )
    return {"value": value}


def _parse_secret_request(params: object) -> SecretRequest:
    if not isinstance(params, dict):
        raise SecretUnavailableError(
            "secrets.resolve params are not an object",
            details={"failure": "resolver_failed"},
        )
    members = cast("dict[str, object]", params)
    name = members.get("name")
    run_id = members.get("run_id")
    domain = members.get("domain")
    if (
        not isinstance(name, str)
        or not isinstance(run_id, str)
        or (domain is not None and not isinstance(domain, str))
    ):
        raise SecretUnavailableError(
            "secrets.resolve params are malformed",
            details={"failure": "resolver_failed"},
        )
    return SecretRequest(name=name, run_id=run_id, domain=domain)


def _redacted_unavailable(error: Exception) -> SecretUnavailableError:
    """Turns any exception from `resolve_secret` into a redacted error.

    Keeps only a string `failure` tag when the caller raised
    `SecretUnavailableError` itself; drops the exception's message, traceback
    and other details so no Secret canary can reach the JSON-RPC error reply
    (ADR 0067).
    """
    failure = "resolver_failed"
    if isinstance(error, SecretUnavailableError):
        raw_failure = error.details.get("failure")
        if isinstance(raw_failure, str) and raw_failure in _SECRET_FAILURES:
            failure = raw_failure
    return SecretUnavailableError(
        "resolve_secret failed",
        details={"failure": failure},
    )
