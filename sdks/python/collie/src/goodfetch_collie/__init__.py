"""Collie's Python SDK (ADRs 0090 and 0095).

Where the main types live (ADR 0117): this package, `goodfetch_collie`,
holds `AsyncCollie` and the sync `Collie`; the Collie client package,
`goodfetch_collie_client`, holds `AsyncRun` and `Run`, the results and the
errors, which this package re-exports; `goodfetch_foxhound` holds Foxhound's.
`goodfetch_collie.testing` opens a Connection over a transport a test holds.

The Connection finds Collie's executable and reads only
`GOODFETCH_COLLIE_ENGINE_PATH`. Importing it reads, starts, patches and logs
nothing (ADR 0078).
"""

from goodfetch_collie_client.connection import EngineInfo
from goodfetch_collie_client.generated.errors import (
    CallCancelledError,
    ClosedError,
    ConnectionOverloadedError,
    DeadlineExceededError,
    EngineExitedError,
    EngineNotFoundError,
    EngineOutputInvalidError,
    LimitRaiseRefusedError,
    LimitReachedError,
    NotInProductError,
    ProtocolVersionUnsupportedError,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.models import (
    Product,
    RunReport,
    RunStarted,
    StartedAs,
)
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.limits import Limits, Money, Prices
from goodfetch_collie_client.run import AsyncRun, Run

from goodfetch_collie.collie import AsyncCollie, Collie, Connecting

__all__ = [
    "AsyncCollie",
    "AsyncRun",
    "CallCancelledError",
    "ClosedError",
    "Collie",
    "Connecting",
    "ConnectionOverloadedError",
    "DeadlineExceededError",
    "EngineExitedError",
    "EngineInfo",
    "EngineNotFoundError",
    "EngineOutputInvalidError",
    "GoodfetchError",
    "LimitRaiseRefusedError",
    "LimitReachedError",
    "Limits",
    "Money",
    "NotInProductError",
    "Prices",
    "Product",
    "ProtocolVersionUnsupportedError",
    "Run",
    "RunReport",
    "RunStarted",
    "SettingInvalidError",
    "StartedAs",
]
