"""Collie's Python SDK (ADRs 0090 and 0095).

Where the main types live (ADR 0117): this package, `goodfetch_collie`,
holds `AsyncCollie` and the sync `Collie`; the Collie client package,
`goodfetch_collie_client`, holds `AsyncRun` and `Run`, the results, the
errors, a listener's `Subscription`, each scope's settings types
(`Settings`, `RunSettings`, `SessionSettings`, `PageSettings`), and
`AsyncConnection` and
`Connection`, which both Products' Connections extend, all of which this
package re-exports;
`goodfetch_foxhound` holds Foxhound's.
`goodfetch_collie.testing` opens a Connection over a transport a test holds.

The Connection reaches a Daemon or finds Collie's executable, reading
`GOODFETCH_URL`, `GOODFETCH_TOKEN`, `GOODFETCH_HOME` and
`GOODFETCH_COLLIE_ENGINE_PATH`. Importing it reads, starts, patches and logs
nothing (ADR 0078).
"""

from goodfetch_collie_client.connection import EngineFallback, EngineInfo
from goodfetch_collie_client.connection_type import AsyncConnection, Connection
from goodfetch_collie_client.engine_choice import EngineChoice, RemoteDaemon
from goodfetch_collie_client.generated.errors import (
    CallCancelledError,
    ClosedError,
    ConnectionOverloadedError,
    DaemonUnreachableError,
    DeadlineExceededError,
    EngineExitedError,
    EngineNotFoundError,
    EngineOutputInvalidError,
    LimitRaiseRefusedError,
    LimitReachedError,
    ListenerFailedError,
    NotInProductError,
    ProtocolVersionUnsupportedError,
    RunConflictError,
    RunEndedError,
    RunIdInvalidError,
    RunInUseError,
    RunStateUnavailableError,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.models import (
    Product,
    RunReport,
    RunStarted,
    StartedAs,
)
from goodfetch_collie_client.generated.settings import (
    PageSettings,
    RunSettings,
    SessionSettings,
    Settings,
)
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.handle import Subscription
from goodfetch_collie_client.limits import Limits, Money, Prices
from goodfetch_collie_client.run import AsyncRun, Run

from goodfetch_collie.collie import AsyncCollie, Collie, Connecting

__all__ = [
    "AsyncCollie",
    "AsyncConnection",
    "AsyncRun",
    "CallCancelledError",
    "ClosedError",
    "Collie",
    "Connecting",
    "Connection",
    "ConnectionOverloadedError",
    "DaemonUnreachableError",
    "DeadlineExceededError",
    "EngineChoice",
    "EngineExitedError",
    "EngineFallback",
    "EngineInfo",
    "EngineNotFoundError",
    "EngineOutputInvalidError",
    "GoodfetchError",
    "LimitRaiseRefusedError",
    "LimitReachedError",
    "Limits",
    "ListenerFailedError",
    "Money",
    "NotInProductError",
    "PageSettings",
    "Prices",
    "Product",
    "ProtocolVersionUnsupportedError",
    "RemoteDaemon",
    "Run",
    "RunConflictError",
    "RunEndedError",
    "RunIdInvalidError",
    "RunInUseError",
    "RunReport",
    "RunSettings",
    "RunStarted",
    "RunStateUnavailableError",
    "SessionSettings",
    "SettingInvalidError",
    "Settings",
    "StartedAs",
    "Subscription",
]
