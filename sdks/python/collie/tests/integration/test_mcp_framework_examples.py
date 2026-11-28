"""Model-free docs example tests for MCP-only frameworks (ADRs 0011, 0070; issue 1384).

Verifies `docs/mcp-frameworks.md` names Collie's install line, names the
`browser` MCP profile, and provides valid model-free examples for the OpenAI
Agents SDK, Pydantic AI, CrewAI, Google ADK, LlamaIndex, and Strands. Runs the
extracted MCP stdio invocation against the real `goodfetch-collie` binary
(connect, list tools, call one) without any model.
"""

from __future__ import annotations

import ast
import json
import re
import subprocess
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest
from goodfetch_test_support import Tool

if TYPE_CHECKING:
    from goodfetch_test_support import TestEnv

REPO_ROOT = Path(__file__).resolve().parents[5]
DOC_PAGE = REPO_ROOT / "docs" / "mcp-frameworks.md"

EXPECTED_FRAMEWORKS: tuple[tuple[str, str], ...] = (
    ("OpenAI Agents SDK", "MCPServerStdio"),
    ("Pydantic AI", "MCPServerStdio"),
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


def _rpc_request(
    proc: subprocess.Popen[str],
    request_id: int,
    method: str,
    params: dict[str, Any],
) -> dict[str, Any]:
    assert proc.stdin is not None
    assert proc.stdout is not None
    payload = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()
    while True:
        line = proc.stdout.readline()
        assert line, "MCP server closed stdout before replying"
        message: dict[str, Any] = json.loads(line)
        if message.get("id") == request_id:
            return message


def _rpc_notify(
    proc: subprocess.Popen[str],
    method: str,
    params: dict[str, Any],
) -> None:
    assert proc.stdin is not None
    payload = {"jsonrpc": "2.0", "method": method, "params": params}
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()


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
        # Every extracted example parses as valid Python without any model key.
        ast.parse(code, filename=f"{DOC_PAGE.name}:{framework}")
        assert client_symbol in code, f"{framework} must use {client_symbol}"
        assert '"mcp"' in code and '"--profile"' in code and '"browser"' in code
        assert "run_start" in code
        assert "OPENAI_API_KEY" not in code
        assert "ANTHROPIC_API_KEY" not in code


@pytest.mark.covers("story:11-247")
def test_mcp_framework_examples_run_model_free_against_real_collie_mcp(
    test_env: TestEnv,
) -> None:
    """Connects, lists tools, and calls a tool on `goodfetch-collie mcp`."""
    examples = _extract_framework_examples()
    collie_bin = test_env.tools.path(Tool.COLLIE)

    # Every one of the six examples launches `goodfetch-collie mcp --profile browser`.
    for framework, _ in EXPECTED_FRAMEWORKS:
        assert '["mcp", "--profile", "browser"]' in examples[framework]

    proc = subprocess.Popen(
        [str(collie_bin), "mcp", "--profile", "browser"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=dict(test_env.env),
    )
    try:
        init_resp = _rpc_request(
            proc,
            1,
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "mcp-framework-doc-test", "version": "0.1.0"},
            },
        )
        assert "result" in init_resp, init_resp
        _rpc_notify(proc, "notifications/initialized", {})

        tools_resp = _rpc_request(proc, 2, "tools/list", {})
        tools: list[dict[str, Any]] = tools_resp["result"]["tools"]
        names = {tool["name"] for tool in tools}
        assert "run_start" in names
        assert "run_finish" in names
        assert "page_lend" not in names
        assert "model_set_price" not in names

        start_resp = _rpc_request(
            proc,
            3,
            "tools/call",
            {"name": "run_start", "arguments": {}},
        )
        assert "result" in start_resp, start_resp
        assert not start_resp["result"].get("isError", False), start_resp
        structured = start_resp["result"].get("structuredContent", {})
        run_id = structured.get("run_id")
        assert isinstance(run_id, str) and run_id

        finish_resp = _rpc_request(
            proc,
            4,
            "tools/call",
            {"name": "run_finish", "arguments": {"run_id": run_id}},
        )
        assert "result" in finish_resp, finish_resp
        assert not finish_resp["result"].get("isError", False), finish_resp
    finally:
        if proc.stdin is not None:
            proc.stdin.close()
        proc.wait(timeout=15)
