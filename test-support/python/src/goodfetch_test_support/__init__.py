"""Goodfetch's shared pytest plugin (ADR 0062).

Its fixtures reach the setup `goodfetch-test-env` prepares, as Rust tests get
it from `TestEnv::new()`; nothing of that setup is rewritten in Python.
"""

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
from goodfetch_test_support.daemon import Daemon, DaemonNotRequested
from goodfetch_test_support.environment import (
    ProcessesAlive,
    Product,
    TestEnv,
    TestEnvError,
)
from goodfetch_test_support.locales import TimeZoneCase, locales, time_zones
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

__all__ = [
    "Canary",
    "CanaryHit",
    "CanaryOutputUnreadable",
    "Daemon",
    "DaemonNotRequested",
    "Encoding",
    "NamedOutput",
    "Placement",
    "ProcessesAlive",
    "Product",
    "ProvingGround",
    "ProvingGroundFixtures",
    "ProvingGroundRoutes",
    "ProvingGroundUnseeded",
    "ProxyCredentials",
    "SchemaError",
    "SecretKind",
    "TestEnv",
    "TestEnvError",
    "TimeZoneCase",
    "Tool",
    "Tools",
    "ToolsError",
    "UnicodeField",
    "UnicodeRecord",
    "locales",
    "proving_ground_routes",
    "redact",
    "search_canaries",
    "time_zones",
    "unicode_record",
    "write_journey_document",
]
