"""Lowest-direct peer-range check for `goodfetch-collie-langchain` (ADR 0070; 1382).

Covers: story:11-244
"""

from __future__ import annotations

import email
import os
import subprocess
import zipfile
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest
from goodfetch_collie_langchain import collie_tools
from goodfetch_test_support.install import System, install_lowest_direct
from goodfetch_test_support.langchain_model import BindToolsChatModel
from langchain_core.messages import AIMessage, HumanMessage

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client.generated.models import Form
    from goodfetch_test_support import TestEnv
    from pydantic import JsonValue

ADAPTER_DIR = Path(__file__).resolve().parents[2]


def _make_minimal_wheel(
    dest_dir: Path,
    name: str,
    version: str,
    requires_dist: tuple[str, ...] = (),
) -> Path:
    """Builds a minimal pure-Python wheel on disk for offline lowest-direct testing."""
    dist_name = name.replace("-", "_")
    wheel_path = dest_dir / f"{dist_name}-{version}-py3-none-any.whl"
    dist_info = f"{dist_name}-{version}.dist-info"
    metadata_lines = [
        "Metadata-Version: 2.1",
        f"Name: {name}",
        f"Version: {version}",
        *[f"Requires-Dist: {req}" for req in requires_dist],
        "",
    ]
    wheel_lines = [
        "Wheel-Version: 1.0",
        "Generator: goodfetch-test",
        "Root-Is-Purelib: true",
        "Tag: py3-none-any",
        "",
    ]
    with zipfile.ZipFile(wheel_path, "w") as archive:
        archive.writestr(f"{dist_name}/__init__.py", f'__version__ = "{version}"\n')
        archive.writestr(f"{dist_info}/METADATA", "\n".join(metadata_lines))
        archive.writestr(f"{dist_info}/WHEEL", "\n".join(wheel_lines))
        archive.writestr(f"{dist_info}/RECORD", "")
    return wheel_path


class _RecordingConnection:
    """Minimal connection stand-in for verifying LangChain agent tool loops."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []

    def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        form: Form | str | None = None,
    ) -> JsonValue:
        del form
        payload: dict[str, Any] = dict(params or {})
        self.calls.append((operation, payload))
        if operation == "run.start":
            return {"run_id": "r_py_peer_1", "lifecycle": "running"}
        if operation == "page.goto":
            return {"url": str(payload.get("url", "")), "status": 200}
        return {"ok": True}


@pytest.mark.covers("story:11-244")
def test_uv_lowest_direct_resolution_installs_and_runs_agent_loop(
    test_env: TestEnv,
    tmp_path: Path,
) -> None:
    """Verifies wheel metadata, `install_lowest_direct` exit 0, and agent tool loop."""
    # 1. Build `goodfetch-collie-langchain`'s wheel offline and check Requires-Dist.
    built_dir = tmp_path / "built"
    built_dir.mkdir()
    build = subprocess.run(
        [
            "uv",
            "build",
            "--wheel",
            "--no-build-isolation",
            "--offline",
            "--out-dir",
            str(built_dir),
            str(ADAPTER_DIR),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert build.returncode == 0, f"uv build failed: {build.stderr}"
    adapter_wheels = list(built_dir.glob("goodfetch_collie_langchain-*.whl"))
    assert len(adapter_wheels) == 1, f"expected one built wheel, found {adapter_wheels}"
    adapter_wheel = adapter_wheels[0]

    with zipfile.ZipFile(adapter_wheel) as archive:
        [meta_name] = [
            name for name in archive.namelist() if name.endswith(".dist-info/METADATA")
        ]
        meta = email.message_from_bytes(archive.read(meta_name))
    requires_dist = list(meta.get_all("Requires-Dist") or ())
    assert any(
        req.replace(" ", "").startswith("langchain-core>=1.0.0,<2")
        for req in requires_dist
    ), f"wheel metadata missing langchain-core>=1.0.0,<2: {requires_dist}"

    # 2. Exercise the adapter through LangChain's `BindToolsChatModel` tool loop.
    conn = _RecordingConnection()
    tools = collie_tools(conn, profile="browser")
    by_name = {tool.name: tool for tool in tools}
    model = BindToolsChatModel(
        messages=iter(
            [
                AIMessage(
                    content="",
                    tool_calls=[
                        {"name": "run_start", "args": {}, "id": "call-1"},
                    ],
                ),
                AIMessage(content="done"),
            ]
        )
    )
    bound = model.bind_tools(tools)
    first = bound.invoke([HumanMessage(content="Start a run")])
    assert isinstance(first, AIMessage)
    assert first.tool_calls and first.tool_calls[0]["name"] == "run_start"
    tool_output = by_name["run_start"].invoke(first.tool_calls[0]["args"])
    assert "r_py_peer_1" in str(tool_output)
    assert conn.calls == [("run.start", {})]

    # 3. Exercise `install_lowest_direct` end-to-end with exit == 0:
    #    - In Release suite (or when GOODFETCH_ADAPTER_WHEELHOUSE is provided),
    #      install the real `goodfetch-collie-langchain` wheel with its full
    #      offline wheelhouse and assert `langchain-core==1.0.0` is resolved.
    #    - In local runs without a full PyPI mirror, prove `install_lowest_direct`
    #      resolves the lowest allowed direct version (1.0.0 over 1.9.0) cleanly
    #      with exit == 0.
    wheelhouse_env = os.environ.get("GOODFETCH_ADAPTER_WHEELHOUSE")
    if os.environ.get("GOODFETCH_TEST_SUITE") == "release" or wheelhouse_env:
        assert wheelhouse_env, (
            "Release suite must set GOODFETCH_ADAPTER_WHEELHOUSE "
            "via ci/release/cell.sh adapters-fetch"
        )
        wheelhouse = Path(wheelhouse_env)
        available = [
            whl
            for whl in sorted(wheelhouse.glob("*.whl"))
            if not whl.name.startswith("goodfetch_collie_langchain-")
        ]
        installed = install_lowest_direct(
            test_env,
            adapter_wheel,
            available,
            system=System.HOST,
        )
        assert installed.install.exit == 0, (
            "install_lowest_direct failed:\n"
            f"{installed.install.stdout}\n{installed.install.stderr}"
        )
        check = installed.run(
            "python",
            "-c",
            (
                "import importlib.metadata; "
                "from goodfetch_collie_langchain import collie_tools; "
                "v = importlib.metadata.version('langchain-core'); "
                "assert v == '1.0.0', f'expected 1.0.0, got {v}'; "
                "print(v)"
            ),
        )
        assert check.exit == 0, f"lowest-version runtime check failed: {check.stderr}"
    else:
        fixture_dir = tmp_path / "fixture-wheels"
        fixture_dir.mkdir()
        low = _make_minimal_wheel(fixture_dir, "gf-peer-fixture", "1.0.0")
        high = _make_minimal_wheel(fixture_dir, "gf-peer-fixture", "1.9.0")
        consumer = _make_minimal_wheel(
            fixture_dir,
            "gf-adapter-fixture",
            "0.1.0",
            requires_dist=("gf-peer-fixture>=1.0.0,<2",),
        )
        installed = install_lowest_direct(
            test_env,
            consumer,
            [low, high],
            system=System.HOST,
        )
        assert installed.install.exit == 0, (
            "install_lowest_direct failed:\n"
            f"{installed.install.stdout}\n{installed.install.stderr}"
        )
        probe = installed.run(
            "python",
            "-c",
            (
                "import importlib.metadata; "
                "v = importlib.metadata.version('gf-peer-fixture'); "
                "assert v == '1.0.0', f'expected 1.0.0, got {v}'"
            ),
        )
        assert probe.exit == 0, probe.stderr
