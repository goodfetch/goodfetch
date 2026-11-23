"""The Connection both Products' Connection types extend (ADRs 0095 and 0099).

A helper a library serving both Products writes takes `AsyncConnection` or
`Connection`, so it accepts either Product's Connection: `AsyncCollie` and
`Collie` extend them, as Foxhound's twins will. Each declares the members
both Products share, and gives the scoped form, which closes the Connection.
TypeScript's `connection.ts` is its twin.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from typing import TYPE_CHECKING

from goodfetch_collie_client.handle import NotGiven, not_given

if TYPE_CHECKING:
    from collections.abc import Iterable, Mapping
    from datetime import timedelta
    from typing import Self

    from pydantic import JsonValue

    from goodfetch_collie_client.connection import EngineInfo
    from goodfetch_collie_client.generated.models import (
        Recording,
        SettingsExplanation,
    )
    from goodfetch_collie_client.generated.settings import RunSettings
    from goodfetch_collie_client.handlerless import CatalogTarget
    from goodfetch_collie_client.limits import Limits
    from goodfetch_collie_client.run import AsyncRun, Run
    from goodfetch_collie_client.run_view import AsyncRunView, RunView

__all__ = ["AsyncConnection", "Connection"]


class AsyncConnection(ABC):
    """A Client's open link to one engine, through which a program reaches its Runs.

    The async API's: `AsyncCollie` extends it.
    """

    @property
    @abstractmethod
    def engine(self) -> EngineInfo:
        """The engine reached, fixed when the Connection opened.

        Read without a round trip.
        """

    @abstractmethod
    async def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
    ) -> AsyncRun:
        """Starts a Run and returns once it is started (ADR 0102).

        Starts a Run its program drives, and answers once it is started. A Run ID
        names the Run for good, and a start under one acts on what the Run index
        holds (ADR 0102): no such Run begins it, `new`; an interrupted Run, stopped,
        failed, or running with no live writer, resumes from its committed state at
        the next run epoch, `resumed`, as `run.resume` does; a finished Run is
        answered as it ended with no handler called, `ended`. A Run live with a
        writer, in this engine or another, fails with `run_in_use`; a Run started
        with other inputs or by another caller with `run_conflict` listing each
        difference; a Run the Run index lists whose state is missing or unreadable
        with `run_state_unavailable`, never starting afresh.

        Experimental: outside semver until it is promoted (ADR 0030).

        `settings` are the Run's own, over the Connection's (ADR 0083).
        Raises `closed` once the Connection is closed.
        """

    @abstractmethod
    async def start_recipe(
        self,
        recipe: str,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> AsyncRun:
        """Starts a Recipe's run and returns once it is started (ADR 0102).

        Starts a Recipe's run, a Run with no handler in the program, and answers once
        it is started. Its Run ID works as `run.start`'s. With `detached`, the Run
        outlives the program that started it: closing its value frees only the
        value, `run.finish` waits for its end and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).

        It takes `start`'s options, a Response `recording` (ADR 0105), the
        Run's own `timeout`, `timeout.run`'s value when left out and none
        for `None`, and `detached`. Raises `closed` once the Connection is
        closed.
        """

    @abstractmethod
    async def start_catalog_run(
        self,
        target: CatalogTarget,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> AsyncRun:
        """Starts a code-free catalog run and returns once it is started (ADR 0036).

        Starts a code-free catalog run, a Run with no handler in the program whose
        verbs come from the catalog (ADR 0036): an Extraction of a page by a schema,
        `extract`, an Agent task, `agent`, or a Plan replayed with no model call,
        `plan_replay`; and answers once it is started. Its Run ID works as
        `run.start`'s. With `detached`, the Run outlives the program that started
        it: closing its value frees only the value, `run.finish` waits for its end
        and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).

        `target` is an `ExtractTarget`, an `AgentTarget` or a
        `PlanReplayTarget`; its options are `start_recipe`'s.
        """

    @abstractmethod
    async def open_run(self, run_id: str) -> AsyncRunView:
        """Opens a view of the Run `run_id` names, with no round trip (ADR 0102).

        The Run is one another program started, such as a detached Run. The
        view takes no writer role, and closing it frees only the view.
        Raises `closed` once the Connection is closed.
        """

    @abstractmethod
    async def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation`, one acting on no value, as the wire's JSON (ADR 0099).

        The way down to an operation the SDK does not generate. `params`
        are sent and the result answered as on the wire, in snake_case.
        `timeout` is sent as the operation's own; cancelling the task
        cancels the call. It is never retried, and raises `closed` once the
        Connection is closed.
        """

    @abstractmethod
    async def explain_settings(
        self, keys: Iterable[str] | None = None
    ) -> SettingsExplanation:
        """What the Connection runs with (ADR 0083), as `config explain` shows it.

        Shows each setting's effective value, the layer that set it and the layer
        that locked it, from the built-in defaults, the Operator policy, user
        configuration, the project configuration given and the run layer: the
        Connection's own settings with the run values given over them, or a live
        Run's as it opened when `run_id` names one; and lists the keys only the
        other Product honours as that Product's.

        It explains each setting under `keys`, each a key or a prefix of keys
        such as `timeout`, every one when left out, with the origin that set
        it too. The Connection's own `settings` are what each Run it starts
        inherits. Raises `closed` once the Connection is closed.
        """

    @abstractmethod
    async def close(self) -> None:
        """Closes the Connection; idempotent.

        Closes every Run it opened first, stopping one still running, cancels
        the calls in flight, and stops an Attached engine, waiting for it.
        Any later call fails with `closed`.
        """

    async def __aenter__(self) -> Self:
        """Holds the Connection for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the Connection."""
        await self.close()


class Connection(ABC):
    """A Client's open link to one engine, through which a program reaches its Runs.

    The sync API's: `Collie` extends it.
    """

    @property
    @abstractmethod
    def engine(self) -> EngineInfo:
        """The engine reached, fixed when the Connection opened.

        Read without a round trip.
        """

    @abstractmethod
    def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
    ) -> Run:
        """Starts a Run and returns once it is started (ADR 0102).

        Starts a Run its program drives, and answers once it is started. A Run ID
        names the Run for good, and a start under one acts on what the Run index
        holds (ADR 0102): no such Run begins it, `new`; an interrupted Run, stopped,
        failed, or running with no live writer, resumes from its committed state at
        the next run epoch, `resumed`, as `run.resume` does; a finished Run is
        answered as it ended with no handler called, `ended`. A Run live with a
        writer, in this engine or another, fails with `run_in_use`; a Run started
        with other inputs or by another caller with `run_conflict` listing each
        difference; a Run the Run index lists whose state is missing or unreadable
        with `run_state_unavailable`, never starting afresh.

        Experimental: outside semver until it is promoted (ADR 0030).

        `settings` are the Run's own, over the Connection's (ADR 0083).
        Raises `closed` once the Connection is closed.
        """

    @abstractmethod
    def start_recipe(
        self,
        recipe: str,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> Run:
        """Starts a Recipe's run, as `AsyncConnection.start_recipe` says.

        Starts a Recipe's run, a Run with no handler in the program, and answers once
        it is started. Its Run ID works as `run.start`'s. With `detached`, the Run
        outlives the program that started it: closing its value frees only the
        value, `run.finish` waits for its end and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).
        """

    @abstractmethod
    def start_catalog_run(
        self,
        target: CatalogTarget,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> Run:
        """Starts a code-free catalog run, as `AsyncConnection.start_catalog_run` says.

        Starts a code-free catalog run, a Run with no handler in the program whose
        verbs come from the catalog (ADR 0036): an Extraction of a page by a schema,
        `extract`, an Agent task, `agent`, or a Plan replayed with no model call,
        `plan_replay`; and answers once it is started. Its Run ID works as
        `run.start`'s. With `detached`, the Run outlives the program that started
        it: closing its value frees only the value, `run.finish` waits for its end
        and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).
        """

    @abstractmethod
    def open_run(self, run_id: str) -> RunView:
        """Opens a view of a Run by its ID, as `AsyncConnection.open_run` says."""

    @abstractmethod
    def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation` as `AsyncConnection.call` does; waits for its result."""

    @abstractmethod
    def explain_settings(
        self, keys: Iterable[str] | None = None
    ) -> SettingsExplanation:
        """What the Connection runs with, as `AsyncConnection.explain_settings` says.

        Shows each setting's effective value, the layer that set it and the layer
        that locked it, from the built-in defaults, the Operator policy, user
        configuration, the project configuration given and the run layer: the
        Connection's own settings with the run values given over them, or a live
        Run's as it opened when `run_id` names one; and lists the keys only the
        other Product honours as that Product's.
        """

    @abstractmethod
    def close(self) -> None:
        """Closes the Connection; idempotent, from any thread.

        Closes every Run it opened first, stopping one still running, cancels
        the calls in flight, and stops an Attached engine, waiting for it.
        Any later call fails with `closed`.
        """

    def __enter__(self) -> Self:
        """Holds the Connection for a `with` block, which closes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the Connection."""
        self.close()
