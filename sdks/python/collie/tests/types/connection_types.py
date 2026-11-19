"""The Connection's types (ADRs 0095 and 0099), checked by pyright and mypy.

A helper a library serving both Products writes, typed with the client
package's `AsyncConnection` or `Connection`, takes `AsyncCollie` or
`Collie`, and refuses any other value; mypy's `--strict` reports an ignore
that no longer silences an error. TypeScript's `connection.test-d.ts` is its
twin.
"""

from typing import assert_type

from goodfetch_collie import (
    AsyncCollie,
    AsyncConnection,
    AsyncRun,
    Collie,
    Connection,
    EngineInfo,
    Run,
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
