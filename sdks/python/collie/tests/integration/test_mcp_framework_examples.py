"""Model-free docs example tests for MCP-only frameworks (ADRs 0011, 0070; issue 1384).

Verifies `docs/mcp-frameworks.md` names Collie's install line, names the
`browser` MCP profile, and provides valid model-free examples for the OpenAI
Agents SDK, Pydantic AI, CrewAI, Google ADK, LlamaIndex, and Strands. Executes
each extracted framework example directly against the real `goodfetch-collie`
binary (connect, list tools, call `run_start`) without any model key.
"""

from __future__ import annotations

import ast
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import Tool

if TYPE_CHECKING:
    from goodfetch_test_support import TestEnv

REPO_ROOT = Path(__file__).resolve().parents[5]
DOC_PAGE = REPO_ROOT / "docs" / "mcp-frameworks.md"
GOOGLE_ADK_DIR = Path(__file__).resolve().parent / "google-adk"
MCP_FRAMEWORKS_DIR = Path(__file__).resolve().parent / "mcp-frameworks"

EXPECTED_FRAMEWORKS: tuple[tuple[str, str], ...] = (
    ("OpenAI Agents SDK", "MCPServerStdio"),
    ("Pydantic AI", "MCPToolset"),
    ("CrewAI", "MCPServerAdapter"),
    ("Google ADK", "MCPToolset"),
    ("LlamaIndex", "BasicMCPClient"),
    ("Strands", "MCPClient"),
)

_SECTION_RE = re.compile(
    r"^## (?P<title>.+?)\n(?P<body>.*?)(?=^## |\Z)",
    re.MULTILINE | re.DOTALL,
)
_PYTHON_BLOCK_RE = re.compile(r"```python\n(?P<code>.*?)\n```", re.DOTALL)


def _extract_framework_examples() -> dict[str, str]:
    text = DOC_PAGE.read_text(encoding="utf-8")
    examples: dict[str, str] = {}
    for match in _SECTION_RE.finditer(text):
        title = match.group("title").strip()
        block = _PYTHON_BLOCK_RE.search(match.group("body"))
        if block is not None:
            examples[title] = block.group("code")
    return examples


@pytest.mark.covers("story:11-247")
def test_mcp_frameworks_doc_page_has_profile_install_line_and_all_six_examples() -> (
    None
):
    """The docs page names the install line, profile, and all 6 model-free examples."""
    text = DOC_PAGE.read_text(encoding="utf-8")
    assert "products: [collie]" in text
    assert "cargo binstall goodfetch-collie" in text
    assert "goodfetch-collie mcp --profile browser" in text

    examples = _extract_framework_examples()
    assert len(examples) == len(EXPECTED_FRAMEWORKS)

    for framework, client_symbol in EXPECTED_FRAMEWORKS:
        assert framework in examples, f"missing example section for {framework}"
        code = examples[framework]
        ast.parse(code, filename=f"{DOC_PAGE.name}:{framework}")
        assert client_symbol in code, f"{framework} must use {client_symbol}"
        assert '"mcp"' in code and '"--profile"' in code and '"browser"' in code
        assert "run_start" in code
        assert "OPENAI_API_KEY" not in code
        assert "ANTHROPIC_API_KEY" not in code


@pytest.mark.covers("story:11-247")
@pytest.mark.parametrize(
    ("framework", "_client_symbol"),
    EXPECTED_FRAMEWORKS,
    ids=[name.lower().replace(" ", "_") for name, _ in EXPECTED_FRAMEWORKS],
)
def test_mcp_framework_example_runs_model_free_against_real_collie_mcp(
    framework: str,
    _client_symbol: str,
    test_env: TestEnv,
    tmp_path: Path,
) -> None:
    """Executes the extracted docs example for `framework` on `goodfetch-collie`."""
    examples = _extract_framework_examples()
    code = examples[framework]
    collie_bin = test_env.tools.path(Tool.COLLIE)

    script_path = tmp_path / f"example_{framework.lower().replace(' ', '_')}.py"
    script_path.write_text(code, encoding="utf-8")

    env = {
        **dict(test_env.env),
        "PATH": os.environ.get("PATH", ""),
        "GOODFETCH_COLLIE_BIN": str(collie_bin),
        # Ensure zero model API keys exist in the child environment.
        "OPENAI_API_KEY": "",
        "ANTHROPIC_API_KEY": "",
        "GOOGLE_API_KEY": "",
    }
    env.pop("OPENAI_API_KEY", None)
    env.pop("ANTHROPIC_API_KEY", None)
    env.pop("GOOGLE_API_KEY", None)

    env_dir = GOOGLE_ADK_DIR if framework == "Google ADK" else MCP_FRAMEWORKS_DIR
    env_python = (
        env_dir
        / ".venv"
        / ("Scripts" if sys.platform == "win32" else "bin")
        / ("python.exe" if sys.platform == "win32" else "python")
    )
    cmd = (
        [str(env_python), str(script_path)]
        if env_python.is_file()
        else [
            "uv",
            "run",
            "--frozen",
            "--offline",
            "--directory",
            str(env_dir),
            "python",
            str(script_path),
        ]
    )

    result = subprocess.run(
        cmd,
        env=env,
        capture_output=True,
        text=True,
        timeout=90,
        check=False,
    )
    assert result.returncode == 0, (
        f"{framework} example failed (exit {result.returncode}):\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )
