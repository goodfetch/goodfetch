"""The Connection's types (ADRs 0095 and 0099), checked by pyright and mypy.

A helper a library serving both Products writes, typed with the client
package's `AsyncConnection` or `Connection`, takes `AsyncCollie` or
`Collie`, and refuses any other value; mypy's `--strict` reports an ignore
that no longer silences an error. It reaches the handler-less starts and
`open_run` too, the view of a Run being its API's `AsyncRunView` or
`RunView`. TypeScript's `connection.test-d.ts` is its twin.
"""

from typing import assert_type

from goodfetch_collie import (
    AsyncCollie,
    AsyncConnection,
    AsyncRun,
    AsyncRunView,
    Collie,
    Connection,
    EngineInfo,
    PlanReplayTarget,
    Run,
    RunView,
)


async def start_on(connection: AsyncConnection) -> AsyncRun:
    """A library's async helper, written once for either Product."""
    return await connection.start()


def start_on_sync(connection: Connection) -> Run:
    """A library's sync helper, written once for either Product."""
    return connection.start()


async def takes_collie(collie: AsyncCollie, sync_collie: Collie, run: AsyncRun) -> None:
    """Each Product's Connection fits its API's helper, and nothing else does."""
    assert_type(await start_on(collie), AsyncRun)
    assert_type(start_on_sync(sync_collie), Run)
    assert_type(collie.engine, EngineInfo)
    await start_on(sync_collie)  # type: ignore[arg-type]
    await start_on(run)  # type: ignore[arg-type]
    start_on_sync(collie)  # type: ignore[arg-type]


async def reaches_a_run_it_does_not_drive(
    connection: AsyncConnection, sync_connection: Connection
) -> None:
    """A helper starts a handler-less Run and opens a view of one by its ID."""
    replay = PlanReplayTarget(plan="plans/checkout.json")
    assert_type(await connection.start_recipe("shop-prices"), AsyncRun)
    assert_type(await connection.start_catalog_run(replay, detached=True), AsyncRun)
    assert_type(await connection.open_run("checkout-replay"), AsyncRunView)
    assert_type(sync_connection.start_recipe("shop-prices"), Run)
    assert_type(sync_connection.start_catalog_run(replay), Run)
    assert_type(sync_connection.open_run("checkout-replay"), RunView)
    await connection.start_catalog_run("plans/checkout.json")  # type: ignore[arg-type]
