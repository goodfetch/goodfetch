"""Goodfetch's shared pytest plugin (ADR 0062).

Its fixtures reach the setup `goodfetch-test-env` prepares, as Rust tests get
it from `TestEnv::new()`; nothing of that setup is rewritten in Python.
"""

from goodfetch_test_support.answering import EngineAnswering
from goodfetch_test_support.canaries import (
    Canary,
    CanaryHit,
    CanaryOutputUnreadable,
    Encoding,
    NamedOutput,
    Placement,
    SecretKind,
    search_canaries,
)
from goodfetch_test_support.case_log import (
    CaseListInvalid,
    CaseLogEntryInvalid,
    ClientCaseRecord,
    ClientCaseUntagged,
)
from goodfetch_test_support.daemon import Daemon, DaemonNotRequested
from goodfetch_test_support.environment import (
    ProcessesAlive,
    Product,
    TestEnv,
    TestEnvError,
    scripted_engine_env,
)
from goodfetch_test_support.journey import (
    EngineVersionUnreported,
    FinishedJourneyRun,
    JourneyInvalid,
    JourneyOther,
    JourneyRun,
    JourneyUnreadable,
    OracleRefused,
    SurfaceUnlisted,
    VariantNotProved,
    journey,
    start_journey_run,
)
from goodfetch_test_support.locales import TimeZoneCase, locales, time_zones
from goodfetch_test_support.play_script import (
    Closed,
    Ending,
    Exited,
    Finished,
    InMemoryTransport,
    NothingToAnswer,
    PlayedMessage,
    PlayedRaw,
    PlayedScript,
    PlayFailed,
    UnexpectedMessage,
    UnusedStep,
    play_script,
)
from goodfetch_test_support.proving_ground import (
    ProvingGround,
    ProvingGroundFixtures,
    ProvingGroundRoutes,
    ProvingGroundUnseeded,
    ProxyCredentials,
    proving_ground_routes,
)
from goodfetch_test_support.result_document import write_journey_document
from goodfetch_test_support.schemas import SchemaError
from goodfetch_test_support.snapshot import redact
from goodfetch_test_support.tools import Tool, Tools, ToolsError
from goodfetch_test_support.unicode_record import (
    UnicodeField,
    UnicodeRecord,
    unicode_record,
)
from goodfetch_test_support.ways import Way, three_ways

__all__ = [
    "Canary",
    "CanaryHit",
    "CanaryOutputUnreadable",
    "CaseListInvalid",
    "CaseLogEntryInvalid",
    "ClientCaseRecord",
    "ClientCaseUntagged",
    "Closed",
    "Daemon",
    "DaemonNotRequested",
    "Encoding",
    "Ending",
    "EngineAnswering",
    "EngineVersionUnreported",
    "Exited",
    "Finished",
    "FinishedJourneyRun",
    "InMemoryTransport",
    "JourneyInvalid",
    "JourneyOther",
    "JourneyRun",
    "JourneyUnreadable",
    "NamedOutput",
    "NothingToAnswer",
    "OracleRefused",
    "Placement",
    "PlayFailed",
    "PlayedMessage",
    "PlayedRaw",
    "PlayedScript",
    "ProcessesAlive",
    "Product",
    "ProvingGround",
    "ProvingGroundFixtures",
    "ProvingGroundRoutes",
    "ProvingGroundUnseeded",
    "ProxyCredentials",
    "SchemaError",
    "SecretKind",
    "SurfaceUnlisted",
    "TestEnv",
    "TestEnvError",
    "TimeZoneCase",
    "Tool",
    "Tools",
    "ToolsError",
    "UnexpectedMessage",
    "UnicodeField",
    "UnicodeRecord",
    "UnusedStep",
    "VariantNotProved",
    "Way",
    "journey",
    "locales",
    "play_script",
    "proving_ground_routes",
    "redact",
    "scripted_engine_env",
    "search_canaries",
    "start_journey_run",
    "three_ways",
    "time_zones",
    "unicode_record",
    "write_journey_document",
]
