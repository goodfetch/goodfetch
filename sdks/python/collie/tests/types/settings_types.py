"""Each scope's settings (ADR 0083), checked by pyright and mypy.

Starting a Run takes `RunSettings`, opening a Session `SessionSettings` and
opening a page `PageSettings`: nested `TypedDict` literals by the wire's
`snake_case` segments, durations as `timedelta`, `None` only where a setting
can be cleared. A key outside its scope, a misspelled or camelCase one, a
setting only configuration sets and a wrong value are each refused; mypy's
`--strict` reports an ignore that no longer silences an error. TypeScript's
`settings.test-d.ts` is its twin.
"""

from datetime import timedelta

from goodfetch_collie import (
    AsyncCollie,
    Collie,
    PageSettings,
    RunSettings,
    SessionSettings,
    Settings,
)

SECOND = timedelta(seconds=1)


async def sets_each_scope(collie: AsyncCollie, sync_collie: Collie) -> None:
    """Each scope takes the settings it can set, and the three states."""
    run = await collie.start(
        settings={
            "timeout": {"navigation": timedelta(seconds=30)},
            "checkpoint": {"interval": None},
            "session": {"source": {"kind": "saved_identity", "name": "shop"}},
        }
    )
    session = await run.new_session(
        settings={
            "browser": {"access_mode": "stealth"},
            "proxy": {"sticky_lifetime": None},
        }
    )
    await session.new_page(settings={"content": {"extractor": "trafilatura"}})
    await run.new_page(settings={})
    sync_run = sync_collie.start(settings={"browser": {"headless": False}})
    sync_run.new_session(settings={"session": {"page_check": None}})
    connection: Settings = {"timeout": {"run": timedelta(hours=1)}}
    starting: RunSettings = {"content": {"extractor": "dom_smoothie"}}
    opening: SessionSettings = {
        "session": {"page_check": {"login_page": {"kind": "css", "selector": "#a"}}}
    }
    paging: PageSettings = {"content": {"extractor": "dom_smoothie"}}
    del connection, starting, opening, paging


def refuses_what_a_scope_cannot_set() -> None:
    """A wrong key, scope or value is a type error, named for its cause."""
    a: RunSettings = {"timeout": {"navigate": SECOND}}  # type: ignore[typeddict-unknown-key]
    b: RunSettings = {"browser": {"accessMode": "stealth"}}  # type: ignore[typeddict-unknown-key]
    c: RunSettings = {"timeout": {"run": SECOND}}  # type: ignore[typeddict-unknown-key]
    d: RunSettings = {"proxy": {"providers": {}}}  # type: ignore[typeddict-unknown-key]
    e: SessionSettings = {"session": {"source": None}}  # type: ignore[typeddict-unknown-key]
    f: PageSettings = {"browser": {"headless": True}}  # type: ignore[typeddict-unknown-key]
    g: RunSettings = {"timeout": {"navigation": 30_000}}  # type: ignore[typeddict-item]
    h: RunSettings = {"timeout": {"navigation": None}}  # type: ignore[typeddict-item]
    i: SessionSettings = {"browser": {"access_mode": "hidden"}}  # type: ignore[typeddict-item]
    del a, b, c, d, e, f, g, h, i


async def each_call_refuses_another_scopes_settings(collie: AsyncCollie) -> None:
    """Each call takes its own scope's type, so a key outside it is refused there."""
    run = await collie.start()
    session = await run.new_session()
    await collie.start(settings={"timeout": {"run": SECOND}})  # type: ignore[arg-type]
    await run.new_session(settings={"session": {"source": None}})  # type: ignore[arg-type]
    await session.new_page(settings={"browser": {"headless": True}})  # type: ignore[arg-type]
