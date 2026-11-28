"""Unit peer-range checks for `goodfetch-collie-langchain` (ADR 0070; issue 1382).

Covers: story:11-244
"""

from __future__ import annotations

import importlib.metadata
import tomllib
from pathlib import Path

import pytest

ADAPTER_DIR = Path(__file__).resolve().parents[2]
REPO_ROOT = Path(__file__).resolve().parents[5]


@pytest.mark.covers("story:11-244")
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
