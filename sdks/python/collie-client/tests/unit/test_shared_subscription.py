"""The Connection's own Subscription, shared by a Session's listeners (ADR 0081).

The Collie tests play one shared Subscription on the Scripted engine; these
cover what a script cannot show: the credits several listeners hold
together, and the warning the engine's falling behind logs. TypeScript's
`shared-subscription.test.ts` is its twin.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import anyio
import anyio.lowlevel
import pytest
from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.host_logging import CLIENT_LOGGER
from goodfetch_collie_client.listeners import (
    EVENTS,
    Listening,
    Order,
    subscribe_published,
)
from goodfetch_collie_client.shared_subscription import SharedSubscription
from pydantic import BaseModel

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping

    from goodfetch_collie_client.goodfetch_error import GoodfetchError
    from goodfetch_collie_client.handle import Subscription


class _Opened(BaseModel):
    page_id: str
    session_id: str


class _Engine:
    """A Connection whose calls stay in flight until cancelled, as `on`'s do."""

    def __init__(self) -> None:
        self.sent: list[tuple[str, object]] = []
        self._next_id = 1
        self._notified: dict[int, Callable[[str, object], None]] = {}

    async def stream(
        self,
        method: str,
        params: Mapping[str, object],
        notified: Callable[[str, object], None],
        *,
        sent: Callable[[int], None] | None = None,
    ) -> object:
        del params  # Each call is told apart by its id.
        call_id = self._next_id
        self._next_id += 1
        self.sent.append((method, call_id))
        self._notified[call_id] = notified
        if sent is not None:
            sent(call_id)
        try:
            await anyio.sleep_forever()
        finally:
            self.sent.append(("$/cancel", call_id))
        return None

    async def notify(self, method: str, params: Mapping[str, object]) -> None:
        self.sent.append((method, params.get("credits")))

    def publish(self, *events: tuple[str, object]) -> None:
        """Sends a batch on the first call, the Connection's Subscription."""
        self._notified[1](
            EVENTS, {"events": [{"event": name, "data": data} for name, data in events]}
        )


def _page(session: str) -> tuple[str, object]:
    return ("session.page", {"page_id": "page-1", "session_id": session})


def _listen(
    engine: _Engine,
    shared: SharedSubscription,
    order: Order,
    session: str,
    listener: Callable[[_Opened], Awaitable[None] | None],
) -> Subscription:
    def mine(data: object) -> bool:
        members = object_members(data)
        return members is not None and members.get("session_id") == session

    def failed(error: Exception) -> None:
        raise AssertionError from error

    def ended(error: GoodfetchError) -> None:
        raise AssertionError from error

    return subscribe_published(
        engine,
        shared,
        "session.on",
        {"session_id": session, "event": "page"},
        mine,
        Listening("session.page", _Opened, listener, failed, ended),
        order,
    ).subscription()


async def _settled() -> None:
    for _ in range(20):
        await anyio.lowlevel.checkpoint()


@pytest.mark.covers("story:11-61", "story:11-62")
@pytest.mark.anyio
async def test_a_removed_listener_gives_back_its_credits_while_others_keep_it() -> None:
    """Its events settle as it leaves: 8 settled grant 8, and the Subscription stays."""
    engine = _Engine()
    shared = SharedSubscription(engine)
    order = Order()
    stuck = anyio.Event()
    heard: list[str] = []

    blocked = _listen(engine, shared, order, "session-1", lambda _: stuck.wait())
    kept = _listen(
        engine, shared, order, "session-2", lambda opened: heard.append(opened.page_id)
    )
    await _settled()
    engine.publish(*[_page("session-1")] * 7, _page("session-2"))
    await _settled()
    granted_while_held = [sent for sent in engine.sent if sent[0] == "$/credit"]
    blocked.close()
    await _settled()
    kept.close()
    await _settled()

    assert granted_while_held == [("$/credit", 16)]
    assert heard == ["page-1"]
    assert engine.sent == [
        ("events.subscribe", 1),
        ("$/credit", 16),
        ("session.on", 2),
        ("session.on", 3),
        ("$/cancel", 2),
        ("$/credit", 8),
        ("$/cancel", 3),
        ("$/cancel", 1),
    ]


@pytest.mark.covers("story:11-62")
@pytest.mark.anyio
async def test_logs_a_warning_when_the_engine_fell_behind(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The events before `events.resync` are gone, which nothing else says."""
    engine = _Engine()
    shared = SharedSubscription(engine)
    listened = _listen(engine, shared, Order(), "session-1", lambda _: None)
    await _settled()

    with caplog.at_level(logging.WARNING, CLIENT_LOGGER):
        engine.publish(("events.resync", {}))
    listened.close()
    await _settled()

    [warned] = [record for record in caplog.records if record.name == CLIENT_LOGGER]
    assert warned.levelno == logging.WARNING
    assert "fell behind" in warned.getMessage()
