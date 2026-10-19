"""Goodfetch's shared pytest plugin (ADR 0062).

Its fixtures reach the setup `goodfetch-test-env` prepares, as Rust tests get
it from `TestEnv::new()`; nothing of that setup is rewritten in Python.
"""

from goodfetch_test_support.environment import (
    ProcessesAlive,
    Product,
    TestEnv,
    TestEnvError,
)
from goodfetch_test_support.proving_ground import (
    ProvingGround,
    ProvingGroundFixtures,
    ProvingGroundUnseeded,
    ProxyCredentials,
)
from goodfetch_test_support.schemas import SchemaError
from goodfetch_test_support.tools import Tool, Tools, ToolsError

__all__ = [
    "ProcessesAlive",
    "Product",
    "ProvingGround",
    "ProvingGroundFixtures",
    "ProvingGroundUnseeded",
    "ProxyCredentials",
    "SchemaError",
    "TestEnv",
    "TestEnvError",
    "Tool",
    "Tools",
    "ToolsError",
]
