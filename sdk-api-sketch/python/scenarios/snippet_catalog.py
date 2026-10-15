"""Snippet for Mapping the catalog to each language and surface (#471, ADR 0099).

The shared `Connection` base classes, the raw catalog call on a connection and on a value, and names spelled by Python's
rule.
"""

from datetime import timedelta

from goodfetch import AsyncConnection, AsyncGoodfetch, AsyncPage, AsyncRoute, Connection, JsonValue


async def engine_version(connection: AsyncConnection) -> str:
    """A library written for both Products takes a connection: `AsyncCollie` and `AsyncFoxhound` both extend it."""
    return connection.engine.version  # a read the SDK answers itself: a property


def engine_version_sync(connection: Connection) -> str:
    return connection.engine.version


async def newer_operation(page: AsyncPage) -> None:
    """The way down (G8): an operation this SDK does not generate, such as one a newer engine minor added.

    `page.emulate_vision_deficiency` is hypothetical here. The page fills its own `page` parameter; `params` are the
    wire's snake_case JSON, where `type` needs no escape.
    """
    await page.call("page.emulate_vision_deficiency", {"type": "deuteranopia"})


async def on_the_connection(connection: AsyncConnection, run_id: str) -> JsonValue:
    """On the connection, `params` names every value itself; the timeout and task cancellation apply (hypothetical)."""
    return await connection.call("run.migrate", {"run": run_id, "to": "daemon"}, timeout=timedelta(minutes=2))


async def wrong_namespace(page: AsyncPage) -> JsonValue:
    return await page.call("run.migrate")  # raises a ProgramError naming `page.*`, before anything is sent


async def names(page: AsyncPage) -> bool:
    """One rule: snake_case members, a trailing underscore on a keyword, `CDPSession` with its initialism in capitals."""

    async def add_trace_header(route: AsyncRoute) -> None:
        await route.continue_(headers={"x-trace": "1"})

    with page.route("**/api/**", add_trace_header):
        await page.wait_for_url("**/orders")
    async with await page.new_cdp_session() as cdp:
        _ = cdp
    response = await page.goto("https://books.toscrape.com/")
    return response is not None and response.ok  # a read without arguments is a property; `is_closed()` stays a method


async def both_products() -> None:
    async with AsyncGoodfetch.connect() as gf:  # placeholder for `AsyncCollie.connect()` until #483
        await engine_version(gf)
