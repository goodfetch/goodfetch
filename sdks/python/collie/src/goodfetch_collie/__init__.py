"""Collie's Python SDK (ADRs 0090 and 0095).

Where the main types live (ADR 0117): this package, `goodfetch_collie`,
holds `AsyncCollie` and the sync `Collie`; the Collie client package,
`goodfetch_collie_client`, holds `AsyncRun` and `Run`, the results, the
errors, a listener's `Subscription`, a sequence's iterators (`AsyncSequence`
and `Sequence`), a Run's Key-value store (`AsyncKeyValueStore` and
`KeyValueStore`), each scope's settings types
(`Settings`, `RunSettings`, `SessionSettings`, `PageSettings`) and what
explaining them answers (`SettingsExplanation`), the
JSON values and the schema written by hand of ADR 0082 (`JSONValue`,
`JSONObject`, `JSONSchema`), and `AsyncConnection` and
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
    ExplainedSetting,
    Product,
    RunOrigin,
    RunReport,
    RunStarted,
    SettingsExplanation,
    SettingsFile,
    SettingsLayer,
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
from goodfetch_collie_client.json_values import JSONObject, JSONValue
from goodfetch_collie_client.key_value_store import AsyncKeyValueStore, KeyValueStore
from goodfetch_collie_client.limits import Limits, Money, Prices
from goodfetch_collie_client.run import AsyncRun, Run
from goodfetch_collie_client.schema import JSONSchema
from goodfetch_collie_client.sequence import AsyncSequence, Sequence

from goodfetch_collie.collie import AsyncCollie, Collie, Connecting

__all__ = [
    "AsyncCollie",
    "AsyncConnection",
    "AsyncKeyValueStore",
    "AsyncRun",
    "AsyncSequence",
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
    "ExplainedSetting",
    "GoodfetchError",
    "JSONObject",
    "JSONSchema",
    "JSONValue",
    "KeyValueStore",
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
    "RunOrigin",
    "RunReport",
    "RunSettings",
    "RunStarted",
    "RunStateUnavailableError",
    "Sequence",
    "SessionSettings",
    "SettingInvalidError",
    "Settings",
    "SettingsExplanation",
    "SettingsFile",
    "SettingsLayer",
    "StartedAs",
    "Subscription",
]
