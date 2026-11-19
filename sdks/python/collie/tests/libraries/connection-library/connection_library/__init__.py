"""A library serving both Products (story 11-180, ADR 0095).

Its `pyproject.toml` takes the Collie client package by a range, so its
import is the program's one copy. Its helper takes the client package's
`AsyncConnection`, and its `except` on the client package's `ClosedError`
catches what a program's Connection raises, which holds only while both
share that copy. TypeScript's `connection-library` package and Rust's
`connection-library` crate are its twins.
"""

from typing import Literal

from goodfetch_collie_client.connection_type import AsyncConnection
from goodfetch_collie_client.generated.errors import ClosedError
from goodfetch_collie_client.generated.models import RunReport


async def finish_one_run(connection: AsyncConnection) -> RunReport | Literal["closed"]:
    """Starts one Run on `connection` and finishes it.

    Answers `"closed"` once the program has closed the Connection.
    """
    try:
        run = await connection.start()
        return await run.finish()
    except ClosedError:
        return "closed"
