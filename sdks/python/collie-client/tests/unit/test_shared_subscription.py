"""The Connection's own Subscription, shared by a Session's listeners (ADR 0081).

The Collie tests play one shared Subscription on the Scripted engine; these
cover what a script cannot show: the credits several listeners hold
together, a listener leaving while others stay, the Subscription ending
unasked, and the warning the engine's falling behind logs. TypeScript's
`shared-subscription.test.ts` is its twin.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import anyio
import anyio.lowlevel
import pytest
from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.generated.errors import EnginePanickedError
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

    from goodfetch_collie_client.goodfetch_error import (
        BaseGoodfetchError,
        GoodfetchError,
    )
    from goodfetch_collie_client.handle import Subscription


class _Opened(BaseModel):
    page_id: str
    session_id: str


class _Engine:
    """A Connection whose calls stay in flight until answered or cancelled."""

    def __init__(self) -> None:
        self.sent: list[tuple[str, object]] = []
        self._next_id = 1
        self._subscription = 0
        self._pages = 0
        self._notified: dict[int, Callable[[str, object], None]] = {}
        self._answered: dict[int, anyio.Event] = {}
        self._errors: dict[int, GoodfetchError] = {}

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
        if method == "events.subscribe":
            self._subscription = call_id
        self.sent.append((method, call_id))
        self._notified[call_id] = notified
        answered = self._answered[call_id] = anyio.Event()
        if sent is not None:
            sent(call_id)
        try:
            await answered.wait()
        except anyio.get_cancelled_exc_class():
            self.sent.append(("$/cancel", call_id))
            raise
        error = self._errors.get(call_id)
        if error is not None:
            raise error
        return None

    async def notify(self, method: str, params: Mapping[str, object]) -> None:
        self.sent.append((method, params.get("credits")))

    def answer(self, call_id: int, error: GoodfetchError | None = None) -> None:
        """Ends a call: it returns, or raises `error`."""
        if error is not None:
            self._errors[call_id] = error
        self._answered[call_id].set()

    def publish(self, *sessions: str) -> None:
        """Sends one page per Session named, `page-1` onwards, in one batch."""
        events: list[tuple[str, object]] = []
        for session in sessions:
            self._pages += 1
            page = {"page_id": f"page-{self._pages}", "session_id": session}
            events.append(("session.page", page))
        self.publish_events(*events)

    def publish_events(self, *events: tuple[str, object]) -> None:
        """Sends a batch on the latest `events.subscribe`."""
        batch = [{"event": name, "data": data} for name, data in events]
        self._notified[self._subscription](EVENTS, {"events": batch})

    def credits(self) -> list[object]:
        """The credits granted so far, in order."""
        return [credits for method, credits in self.sent if method == "$/credit"]


class _Listeners:
    """Session listeners on one Connection's Subscription, and what they heard."""

    def __init__(self, engine: _Engine) -> None:
        self._engine = engine
        self._shared = SharedSubscription(engine)
        self._order = Order()
        self.failures: list[Exception | BaseGoodfetchError] = []
        self.endings: list[BaseGoodfetchError] = []

    def listen(
        self, session: str, listener: Callable[[_Opened], Awaitable[None] | None]
    ) -> Subscription:
        def mine(data: object) -> bool:
            members = object_members(data)
            return members is not None and members.get("session_id") == session

        return subscribe_published(
            self._engine,
            self._shared,
            "session.on",
            {"session_id": session, "event": "page"},
            mine,
            Listening(
                "session.page",
                _Opened,
                listener,
                self.failures.append,
                self.endings.append,
            ),
            self._order,
        ).subscription()


async def _settled() -> None:
    for _ in range(20):
        await anyio.lowlevel.checkpoint()


@pytest.mark.covers("story:11-61", "story:11-62")
@pytest.mark.anyio
async def test_a_removed_listener_gives_back_its_credits_while_others_keep_it() -> None:
    """Its events settle as it leaves: 8 settled grant 8, and the Subscription stays."""
    engine = _Engine()
    listeners = _Listeners(engine)
    stuck = anyio.Event()
    heard: list[str] = []

    blocked = listeners.listen("session-1", lambda _: stuck.wait())
    kept = listeners.listen("session-2", lambda opened: heard.append(opened.page_id))
    await _settled()
    engine.publish(*["session-1"] * 7, "session-2")
    await _settled()
    granted_while_held = engine.credits()
    blocked.close()
    await _settled()
    kept.close()
    await _settled()
    stuck.set()

    assert granted_while_held == [16]
    assert heard == ["page-8"]
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
    assert listeners.failures == []
    assert listeners.endings == []


@pytest.mark.covers("story:11-63")
@pytest.mark.anyio
async def test_grants_an_event_back_once_each_listener_that_took_it_is_through() -> (
    None
):
    """Two listeners of one Session: the slower one holds the credits of both."""
    engine = _Engine()
    listeners = _Listeners(engine)
    stuck = anyio.Event()

    listeners.listen("session-1", lambda _: stuck.wait())
    listeners.listen("session-1", lambda _: None)
    await _settled()
    engine.publish(*["session-1"] * 8)
    await _settled()
    granted_while_held = engine.credits()
    stuck.set()
    await _settled()

    assert granted_while_held == [16]
    assert engine.credits() == [16, 8]


@pytest.mark.covers("story:11-63")
@pytest.mark.anyio
async def test_a_listener_that_throws_leaves_while_the_others_keep_listening() -> None:
    """It is removed and its value hears why; the Subscription stays for the rest."""
    engine = _Engine()
    listeners = _Listeners(engine)
    bug = ValueError("the program's bug")
    heard: list[str] = []

    def throws(_: _Opened) -> None:
        raise bug

    listeners.listen("session-1", throws)
    kept = listeners.listen("session-1", lambda opened: heard.append(opened.page_id))
    await _settled()
    engine.publish("session-1")
    await _settled()
    engine.publish("session-1")
    await _settled()
    sent_while_kept = list(engine.sent)
    kept.close()
    await _settled()

    assert listeners.failures == [bug]
    assert heard == ["page-1", "page-2"]
    assert ("$/cancel", 2) in sent_while_kept
    assert ("$/cancel", 1) not in sent_while_kept


@pytest.mark.covers("story:11-62")
@pytest.mark.anyio
async def test_leaves_once_its_call_is_answered_and_still_delivers_what_it_took() -> (
    None
):
    """The engine ended `on`: the last listener cancels the Subscription."""
    engine = _Engine()
    listeners = _Listeners(engine)
    stuck = anyio.Event()
    heard: list[str] = []

    async def slow(opened: _Opened) -> None:
        await stuck.wait()
        heard.append(opened.page_id)

    listeners.listen("session-1", slow)
    await _settled()
    engine.publish("session-1", "session-1")
    await _settled()
    engine.answer(2)
    await _settled()
    stuck.set()
    await _settled()

    assert heard == ["page-1", "page-2"]
    assert engine.sent == [
        ("events.subscribe", 1),
        ("$/credit", 16),
        ("session.on", 2),
        ("$/cancel", 1),
    ]
    assert listeners.endings == []


@pytest.mark.covers("story:11-62")
@pytest.mark.anyio
async def test_ends_each_listener_with_the_error_that_ended_the_subscription() -> None:
    """Each value hears it; the next listener sends another Subscription."""
    engine = _Engine()
    listeners = _Listeners(engine)
    crash = EnginePanickedError("the engine's bug")

    listeners.listen("session-1", lambda _: None)
    listeners.listen("session-2", lambda _: None)
    await _settled()
    engine.answer(1, crash)
    await _settled()
    ended = list(engine.sent)
    listeners.listen("session-1", lambda _: None)
    await _settled()

    assert listeners.endings == [crash, crash]
    assert ended == [
        ("events.subscribe", 1),
        ("$/credit", 16),
        ("session.on", 2),
        ("session.on", 3),
        ("$/cancel", 2),
        ("$/cancel", 3),
    ]
    assert engine.sent[len(ended) :] == [
        ("events.subscribe", 4),
        ("$/credit", 16),
        ("session.on", 5),
    ]


@pytest.mark.covers("story:11-62")
@pytest.mark.anyio
async def test_logs_a_warning_when_the_engine_fell_behind(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The events before `events.resync` are gone, which nothing else says."""
    engine = _Engine()
    listeners = _Listeners(engine)
    listened = listeners.listen("session-1", lambda _: None)
    await _settled()

    with caplog.at_level(logging.WARNING, CLIENT_LOGGER):
        engine.publish_events(("events.resync", {}))
    listened.close()
    await _settled()

    [warned] = [record for record in caplog.records if record.name == CLIENT_LOGGER]
    assert warned.levelno == logging.WARNING
    assert listeners.endings == []
