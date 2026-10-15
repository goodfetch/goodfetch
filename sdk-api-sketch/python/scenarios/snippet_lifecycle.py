"""Snippet for Entry points and lifecycle (#465): connect, open a Run, borrow the user's page, close in scope."""

import logging

from goodfetch_collie import AsyncCollie, BorrowableAsyncPage, EngineAddress, Collie

logging.basicConfig(level=logging.INFO)  # the host configures logging; Collie's loggers follow it


async def add_step(playwright_page: BorrowableAsyncPage) -> None:
    async with AsyncCollie.connect() as collie, await collie.start() as run:
        async with await run.borrow(playwright_page) as page:
            _ = page  # the AI step itself is #473's
    # page, run and collie are closed in that order; the user's page stays open


async def connection_without_scope() -> None:
    collie = await AsyncCollie.connect(engine=EngineAddress(url="https://daemon.example:7443", token="t"))
    try:
        run = await collie.start()
        await run.new_page()
    finally:
        await collie.close()  # closes the Run it opened; the Daemon keeps running


def in_a_sync_program() -> None:
    with Collie.connect() as collie, collie.start() as run:
        run.new_page()
