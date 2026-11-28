"""Each scope's settings (ADR 0083), checked by pyright and mypy.

Connecting takes `Settings`, starting a Run `RunSettings`, opening a Session
`SessionSettings` and opening a page `PageSettings`: nested `TypedDict`
literals by the wire's `snake_case` segments, durations as `timedelta`,
`None` only where a setting can be cleared. A key outside its scope, a
misspelled or camelCase one, a setting only configuration sets and a wrong
value are each refused; mypy's `--strict` reports an ignore that no longer
silences an error. TypeScript's `settings.test-d.ts` is its twin.
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
    browser_setup: SessionSettings = {
        "browser": {
            "viewport": {"width": 800, "height": 600},
            "device_scale_factor": 1.5,
            "locale": "fr-FR",
            "timezone_id": "Europe/Paris",
            "color_scheme": "dark",
            "java_script_enabled": False,
            "geolocation": {"latitude": 48.8566, "longitude": 2.3522, "accuracy": 10.5},
            "permissions": ["geolocation"],
            "headless": True,
            "block_resources": ["image"],
            "service_workers": "block",
        },
        "network": {
            "throttle": {
                "offline": False,
                "latency": 40,
                "download_bytes_per_second": 500_000,
                "upload_bytes_per_second": 250_000,
            }
        },
    }
    session = await run.new_session(settings=browser_setup)
    await session.new_page(settings={"content": {"extractor": "trafilatura"}})
    await run.new_page(settings={})
    sync_run = sync_collie.start(settings={"browser": {"headless": False}})
    sync_run.new_session(settings=browser_setup)
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
    e_run: SessionSettings = {"timeout": {"navigation": SECOND}}  # type: ignore[typeddict-unknown-key]
    f: PageSettings = {"browser": {"headless": True}}  # type: ignore[typeddict-unknown-key]
    f_vp: PageSettings = {"browser": {"viewport": {"width": 800, "height": 600}}}  # type: ignore[typeddict-unknown-key]
    f_th: PageSettings = {"network": {"throttle": {"offline": True}}}  # type: ignore[typeddict-unknown-key]
    g: RunSettings = {"timeout": {"navigation": 30_000}}  # type: ignore[typeddict-item]
    h: RunSettings = {"timeout": {"navigation": None}}  # type: ignore[typeddict-item]
    i: SessionSettings = {"browser": {"access_mode": "hidden"}}  # type: ignore[typeddict-item]
    del a, b, c, d, e, e_run, f, f_vp, f_th, g, h, i


async def each_call_refuses_another_scopes_settings(collie: AsyncCollie) -> None:
    """Each call takes its own scope's type, so a key outside it is refused there."""
    run = await collie.start()
    session = await run.new_session()
    await collie.start(settings={"timeout": {"run": SECOND}})  # type: ignore[arg-type]
    await run.new_session(settings={"session": {"source": None}})  # type: ignore[arg-type]
    await run.new_session(settings={"timeout": {"navigation": SECOND}})  # type: ignore[arg-type]
    await session.new_page(settings={"browser": {"headless": True}})  # type: ignore[arg-type]
    await session.new_page(settings={"browser": {"viewport": None}})  # type: ignore[arg-type]


async def refuses_a_key_only_foxhound_carries(collie: AsyncCollie) -> None:
    """A setting only Foxhound carries is no Collie key (ADR 0095)."""
    a: RunSettings = {"politeness": {"domain": {"concurrency": 2}}}  # type: ignore[typeddict-unknown-key]
    b: Settings = {"crawl": {"max_requests": 10}}  # type: ignore[typeddict-unknown-key]
    del a, b
    await collie.start(settings={"politeness": {"domain": {"concurrency": 2}}})  # type: ignore[arg-type]


async def connects_with_the_connections_settings() -> None:
    """`connect()` takes the Connection's settings and refuses a key no code sets."""
    await AsyncCollie.connect(settings={"timeout": {"run": timedelta(hours=1)}})
    Collie.connect(settings={"checkpoint": {"interval": None}}).close()
    await AsyncCollie.connect(settings={"proxy": {"providers": {}}})  # type: ignore[arg-type]
