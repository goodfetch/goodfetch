"""The import cost of `goodfetch_foxhound` (ADR 0071; K09).

A fresh interpreter, in the test's environment, imports the package and
prints each top-level package outside the standard library the import
loaded; the snapshot is that counted set. Collie's twin is
`test_importing_the_sdk_loads_no_adapter_ai_provider_or_browser_driver` in
Collie's `test_collie_attached.py`.
"""

from __future__ import annotations

import json
import subprocess
import sys
from typing import TYPE_CHECKING

import pytest
from inline_snapshot import snapshot

if TYPE_CHECKING:
    from goodfetch_test_support import TestEnv

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long the importing interpreter may take."""


@pytest.mark.covers("story:11-1010")
def test_importing_the_sdk_loads_no_adapter_ai_provider_or_browser_driver(
    test_env: TestEnv,
) -> None:
    """Each package outside the standard library the import loads.

    No adapter, AI provider or Playwright is among them. `sysconfig`'s data
    module, named after the platform, is the standard library's own.
    """
    loaded = subprocess.run(
        [
            sys.executable,
            "-c",
            "import json, sys\n"
            "before = set(sys.modules)\n"
            "import goodfetch_foxhound\n"
            "top = {name.partition('.')[0] for name in set(sys.modules) - before}\n"
            "print(json.dumps(sorted(\n"
            "    name for name in top - set(sys.stdlib_module_names)\n"
            "    if not name.startswith('_sysconfigdata_')\n"
            ")))\n",
        ],
        capture_output=True,
        cwd=test_env.folder,
        env=dict(test_env.env),
        timeout=PROGRAM_DEADLINE_SECONDS,
        check=True,
    )

    assert json.loads(loaded.stdout) == snapshot(
        [
            "annotated_types",
            "anyio",
            "goodfetch_collie_client",
            "goodfetch_foxhound",
            "pydantic",
            "pydantic_core",
            "sniffio",
            "typing_extensions",
            "typing_inspection",
            "websockets",
        ]
    )
