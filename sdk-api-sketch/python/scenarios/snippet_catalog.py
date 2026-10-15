"""Snippet for Mapping the catalog to each language and surface (#471, ADR 0099).

The shared `Connection` base classes, the raw catalog call on a connection and on a value, and names spelled by Python's
rule.
"""

from datetime import timedelta

from goodfetch import AsyncConnection, AsyncGoodfetch, AsyncPage, AsyncRoute, AsyncRun, Connection, JsonValue


async def engine_version(connection: AsyncConnection) -> str:
    """A library written for both Products takes a connection: `AsyncCollie` and `AsyncFoxhound` both extend it."""
    return connection.engine.version  # a read the SDK answers itself: a property


def engine_version_sync(connection: Connection) -> str:
    return connection.engine.version


async def report_progress(run: AsyncRun, done: int) -> None:
    """The way down (G8): `run.set_status_message` (area 3's RT-15) is not generated here, so it goes raw.

    The Run fills its own `run` parameter; `params` are the wire's snake_case JSON.
    """
    await run.call("run.set_status_message", {"message": f"{done} pages done"})


async def newer_operation(connection: AsyncConnection, run: AsyncRun) -> JsonValue:
    """On the connection, `params` names every value itself; the timeout and task cancellation apply."""
    return await connection.call("run.checkpoint", {"run": run.id}, timeout=timedelta(minutes=2))


async def wrong_namespace(page: AsyncPage) -> JsonValue:
    return await page.call("run.checkpoint")  # raises a ProgramError naming `page.*`, before anything is sent


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
