"""A view of a Run another program started (ADR 0102), such as a detached one.

A Connection's `open_run()` opens it by the Run's ID with no round trip. It
takes no writer role, and closing it frees only the view: the Run goes on.
TypeScript's `run-view.ts` is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from typing import Self

__all__ = ["AsyncRunView", "RunView"]


class AsyncRunView:
    """A Run named by its Run ID, opened as a view of the async API."""

    def __init__(self, run_id: str) -> None:
        """Takes the Run ID a Connection's `open_run()` was given."""
        self._id = run_id

    @property
    def id(self) -> str:
        """The Run's ID, which names it for good. Read without a round trip."""
        return self._id

    async def close(self) -> None:
        """Closes the view; idempotent. Frees only the view: the Run goes on."""

    async def __aenter__(self) -> Self:
        """Holds the view for an `async with` block, which closes it."""
        return self

    async def __aexit__(self, *_: object) -> None:
        """Closes the view."""
        await self.close()


class RunView:
    """A Run named by its Run ID, opened as a view of the sync API."""

    def __init__(self, view: AsyncRunView) -> None:
        """Takes the view its Connection opened on its private loop."""
        self._view = view

    @property
    def id(self) -> str:
        """The Run's ID, which names it for good. Read without a round trip."""
        return self._view.id

    def close(self) -> None:
        """Closes the view; idempotent. Frees only the view: the Run goes on."""

    def __enter__(self) -> Self:
        """Holds the view for a `with` block, which closes it."""
        return self

    def __exit__(self, *_: object) -> None:
        """Closes the view."""
        self.close()
