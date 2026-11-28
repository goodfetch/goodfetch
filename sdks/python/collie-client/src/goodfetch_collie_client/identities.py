"""A Connection's Saved identities (ADRs 0015, 0035, 0056 and 0099; ticket #736).

`connection.identities`: a sub-object reached without I/O exposing `list`,
`import_`, `delete`, and `export` (as ADR 0015's `age`-encrypted bundle — no
SDK writes clear-text `storageState`). TypeScript's `identities.ts` is its twin.
"""

from __future__ import annotations

import functools
from typing import TYPE_CHECKING, Protocol, TypeVar

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.connection import closed_error
from goodfetch_collie_client.generated.models import (
    IdentityDeleted,
    IdentityDeleteParams,
    IdentityExported,
    IdentityExportParams,
    IdentityImportParams,
    IdentityList,
    IdentityListParams,
    SavedIdentityInfo,
)
from goodfetch_collie_client.markers import warn_experimental
from goodfetch_collie_client.portal import LoopStopped

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable

    from pydantic import BaseModel

    from goodfetch_collie_client.portal import PrivateLoop

__all__ = [
    "AsyncIdentities",
    "Identities",
]

_Result = TypeVar("_Result")
_TIMEOUT_SETTING = "timeout.wait"


class IdentitiesHost(Protocol):
    """What a Connection's Saved identities reach the engine through: its Connection."""

    async def call(
        self, operation: str, params: BaseModel, *, timeout_setting: str | None = None
    ) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...

    def ensure_open(self, operation: str) -> None:
        """Raises `closed` once the Connection's close has begun."""
        ...


class AsyncIdentities:
    """A Connection's Saved identities in the async API: `connection.identities`.

    Each call acts on this Product major's encrypted Saved identity store. A
    program never constructs one.
    """

    def __init__(self, host: IdentitiesHost) -> None:
        """Takes the Connection its calls go through."""
        self._host = host

    async def list(self) -> list[SavedIdentityInfo]:
        """Lists every non-expired Saved identity in this Product major's store.

        Lists every non-expired Saved identity in this Product major's store,
        with its metadata and domains, never secret values (ADR 0035).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("identity.list")
        return await _list(self._host)

    async def import_(self, params: IdentityImportParams) -> SavedIdentityInfo:
        """Imports cookies and storage into the encrypted Saved identity store.

        Imports cookies and storage from a Session source (`state_file`,
        `cookie_header`, etc.) into the encrypted per-major Saved identity store
        under `name`, replacing any existing identity of that name under the
        install-wide lock and recording the import in the Audit log (ADRs 0035
        and 0056).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("identity.import")
        return await _import(self._host, params)

    async def delete(self, name: str) -> IdentityDeleted:
        """Deletes the named Saved identity from this Product major's store.

        Deletes the named Saved identity from this Product major's encrypted
        store, wiping its secret fields and recording the deletion in the Audit
        log (ADR 0035).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("identity.delete")
        return await _delete(self._host, name)

    async def export(self, params: IdentityExportParams) -> IdentityExported:
        """Exports the named Saved identity as an `age`-encrypted bundle.

        Exports the named Saved identity as an `age`-encrypted `session-export`
        bundle to an `age` public key or a passphrase Secret reference,
        preserving its Access mode, Identity profile seed, proxy binding and
        machine binding, and recording the export in the Audit log (ADRs 0015,
        0035 and 0056).

        No SDK writes clear-text `storageState` (ADR 0056).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("identity.export")
        return await _export(self._host, params)


class Identities:
    """A Connection's Saved identities in the sync API, as `AsyncIdentities` are.

    A program never constructs one.
    """

    def __init__(self, loop: PrivateLoop, host: IdentitiesHost) -> None:
        """Takes the Connection's private loop and the Connection it calls through."""
        self._loop = loop
        self._host = host

    def list(self) -> list[SavedIdentityInfo]:
        """Lists every non-expired Saved identity in this Product major's store.

        Lists every non-expired Saved identity in this Product major's store,
        with its metadata and domains, never secret values (ADR 0035).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "identity.list"
        warn_experimental(operation)
        return self._on_loop(operation, functools.partial(_list, self._host))

    def import_(self, params: IdentityImportParams) -> SavedIdentityInfo:
        """Imports cookies and storage into the encrypted Saved identity store.

        Imports cookies and storage from a Session source (`state_file`,
        `cookie_header`, etc.) into the encrypted per-major Saved identity store
        under `name`, replacing any existing identity of that name under the
        install-wide lock and recording the import in the Audit log (ADRs 0035
        and 0056).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "identity.import"
        warn_experimental(operation)
        return self._on_loop(operation, functools.partial(_import, self._host, params))

    def delete(self, name: str) -> IdentityDeleted:
        """Deletes the named Saved identity from this Product major's store.

        Deletes the named Saved identity from this Product major's encrypted
        store, wiping its secret fields and recording the deletion in the Audit
        log (ADR 0035).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "identity.delete"
        warn_experimental(operation)
        return self._on_loop(operation, functools.partial(_delete, self._host, name))

    def export(self, params: IdentityExportParams) -> IdentityExported:
        """Exports the named Saved identity as an `age`-encrypted bundle.

        Exports the named Saved identity as an `age`-encrypted `session-export`
        bundle to an `age` public key or a passphrase Secret reference,
        preserving its Access mode, Identity profile seed, proxy binding and
        machine binding, and recording the export in the Audit log (ADRs 0015,
        0035 and 0056).

        No SDK writes clear-text `storageState` (ADR 0056).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "identity.export"
        warn_experimental(operation)
        return self._on_loop(operation, functools.partial(_export, self._host, params))

    def _on_loop(
        self, operation: str, function: Callable[[], Awaitable[_Result]]
    ) -> _Result:
        """Runs `function` on the private loop; `closed` once the loop stopped."""
        try:
            return self._loop.call(function)
        except LoopStopped:
            raise closed_error(operation) from None


async def _list(host: IdentitiesHost) -> list[SavedIdentityInfo]:
    """Sends `identity.list`, unless the Connection is closed."""
    host.ensure_open("identity.list")
    listed = await host.call(
        "identity.list",
        IdentityListParams.model_construct(),
        timeout_setting=_TIMEOUT_SETTING,
    )
    return result_of(IdentityList, listed).identities


async def _import(
    host: IdentitiesHost, params: IdentityImportParams
) -> SavedIdentityInfo:
    """Sends `identity.import`, unless the Connection is closed."""
    host.ensure_open("identity.import")
    return result_of(
        SavedIdentityInfo,
        await host.call("identity.import", params, timeout_setting=_TIMEOUT_SETTING),
    )


async def _delete(host: IdentitiesHost, name: str) -> IdentityDeleted:
    """Sends `identity.delete`, unless the Connection is closed."""
    host.ensure_open("identity.delete")
    params = IdentityDeleteParams.model_construct(name=name)
    return result_of(
        IdentityDeleted,
        await host.call("identity.delete", params, timeout_setting=_TIMEOUT_SETTING),
    )


async def _export(
    host: IdentitiesHost, params: IdentityExportParams
) -> IdentityExported:
    """Sends `identity.export`, unless the Connection is closed."""
    host.ensure_open("identity.export")
    return result_of(
        IdentityExported,
        await host.call("identity.export", params, timeout_setting=_TIMEOUT_SETTING),
    )
