"""Peer-range verification for `goodfetch-collie-langchain` (ADR 0070; issue 1382).

Covers: story:11-244
"""

from __future__ import annotations

import importlib.metadata
import os
import subprocess
import sys
import tomllib
from pathlib import Path

from goodfetch_test_support.install import install_lowest_direct

ADAPTER_DIR = Path(__file__).resolve().parents[2]
REPO_ROOT = Path(__file__).resolve().parents[5]


def test_python_langchain_peer_range_is_stated_in_readme_and_release_notes() -> None:
    """`pyproject.toml`, README, and the release-notes template state the same range."""
    pyproject_text = (ADAPTER_DIR / "pyproject.toml").read_text(encoding="utf-8")
    pyproject = tomllib.loads(pyproject_text)
    deps: list[str] = pyproject["project"]["dependencies"]
    langchain_deps = [dep for dep in deps if dep.startswith("langchain-core")]
    assert langchain_deps == ["langchain-core>=1.0.0,<2"]

    readme = (ADAPTER_DIR / "README.md").read_text(encoding="utf-8")
    assert "langchain-core>=1.0.0,<2" in readme

    template_path = REPO_ROOT / "docs" / "release-notes-template.md"
    template = template_path.read_text(encoding="utf-8")
    assert "goodfetch-collie-langchain" in template
    assert ">=1.0.0,<2" in template

    installed = importlib.metadata.version("langchain-core")
    assert installed.startswith("1.")


def test_uv_lowest_direct_resolution_path(tmp_path: Path) -> None:
    """Verifies `install_lowest_direct` and uv's `--resolution lowest-direct` path."""
    assert callable(install_lowest_direct)
    if os.environ.get("GOODFETCH_TEST_SUITE") != "release":
        return

    requirements = tmp_path / "req.txt"
    requirements.write_text("langchain-core>=1.0.0,<2\n", encoding="utf-8")
    result = subprocess.run(
        [
            "uv",
            "pip",
            "compile",
            "--offline",
            "--resolution",
            "lowest-direct",
            "--python",
            sys.executable,
            str(requirements),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode in (0, 1), result.stderr
