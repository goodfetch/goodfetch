"""Unit tests for `goodfetch-collie-langchain` (ADRs 0011, 0070, 0071, 0095, 0099).

Covers `ToolsUnitTests` from pinned `langchain-tests` on every generated Python
tool, converted tool snapshots per profile, kept-off operation/granting-param
checks, form `"tool"` + no-retry checks, and a clean-process Python import
snapshot proving no AI provider or Playwright loads on import.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING, Any, cast

import anyio
import pytest
from goodfetch_collie_client.generated.errors import ProgramError
from goodfetch_collie_client.generated.mcp_tools import (
    COLLIE_MCP_PROFILES,
    COLLIE_MCP_TOOLS,
    KEPT_OFF_OPERATIONS,
    McpProfile,
    McpToolSpec,
)
from goodfetch_collie_langchain import CollieTool, collie_tools
from langchain_core.utils.function_calling import convert_to_openai_tool
from langchain_tests.unit_tests import ToolsUnitTests

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client.generated.models import Form
    from pydantic import JsonValue

ALL_PROFILES: tuple[McpProfile, ...] = ("browser", "agent", "run", "code", "full")
SNAPSHOT_DIR = Path(__file__).parent / "snapshots"
TOOLS_SNAPSHOT = SNAPSHOT_DIR / "tools_snapshot.json"


class _StubConnection:
    """In-memory stub of Collie's raw `call` for unit tests."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, Mapping[str, JsonValue] | None, str | None]] = []

    def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        form: Form | str | None = None,
    ) -> JsonValue:
        self.calls.append((operation, params, None if form is None else str(form)))
        if operation == "page.goto":
            raise ProgramError(
                "invalid setting",
                code="setting_not_applicable",
                group="program",
                retryable=True,
                operation=operation,
                fix="Remove `mcp.oauth.issuer` from the run settings.",
            )
        return {"run_id": "r1"}


def _example_value(prop_schema: Mapping[str, Any]) -> object:
    raw_enum: object = prop_schema.get("enum")
    if isinstance(raw_enum, list):
        enum_vals = cast("list[object]", raw_enum)
        if enum_vals:
            return enum_vals[0]
    schema_type = prop_schema.get("type")
    if schema_type == "string":
        return "example"
    if schema_type == "integer":
        return 1
    if schema_type == "number":
        return 1.0
    if schema_type == "boolean":
        return True
    if schema_type == "array":
        return []
    if schema_type == "object":
        return {}
    return "example"


def _example_params(spec: McpToolSpec) -> dict[str, Any]:
    raw_props: object = spec.input_schema.get("properties")
    properties: dict[str, Any] = (
        cast("dict[str, Any]", raw_props) if isinstance(raw_props, dict) else {}
    )
    raw_required: object = spec.input_schema.get("required")
    required: list[str] = (
        [item for item in cast("list[object]", raw_required) if isinstance(item, str)]
        if isinstance(raw_required, list)
        else []
    )
    return {key: _example_value(properties.get(key, {})) for key in required}


def _make_unit_test_class(spec: McpToolSpec) -> type[ToolsUnitTests]:
    connection = _StubConnection()
    example = _example_params(spec)

    @pytest.mark.covers("adapter:langchain", "story:11-242")
    class _GeneratedToolUnitTests(ToolsUnitTests):
        @property
        def tool_constructor(self) -> type[CollieTool]:
            return CollieTool

        @property
        def tool_constructor_params(self) -> dict[str, Any]:
            return {"connection": connection, "spec": spec}

        @property
        def tool_invoke_params_example(self) -> dict[str, Any]:
            return dict(example)

    _GeneratedToolUnitTests.__name__ = f"TestToolUnit_{spec.name}"
    _GeneratedToolUnitTests.__qualname__ = _GeneratedToolUnitTests.__name__
    return _GeneratedToolUnitTests


# Register one `ToolsUnitTests` subclass for every generated Collie Python tool.
for _tool_name, _tool_spec in COLLIE_MCP_TOOLS.items():
    globals()[f"TestToolUnit_{_tool_name}"] = _make_unit_test_class(_tool_spec)


@pytest.mark.covers("adapter:langchain", "story:11-242")
def test_converted_tools_snapshot_per_profile() -> None:
    """Snapshots the converted OpenAI/LangChain tools for every Collie profile."""
    connection = _StubConnection()
    summary: dict[str, list[dict[str, Any]]] = {}
    for profile in ALL_PROFILES:
        tools = collie_tools(connection, profile=profile)
        entries: list[dict[str, Any]] = []
        for item in tools:
            converted = convert_to_openai_tool(item)
            fn = converted["function"]
            raw_params: object = fn.get("parameters", {})
            params_dict = (
                cast("dict[str, Any]", raw_params)
                if isinstance(raw_params, dict)
                else {}
            )
            entries.append(
                {
                    "name": fn["name"],
                    "description": fn.get("description", ""),
                    "read_only": (item.metadata or {}).get("read_only"),
                    "operation_class": (item.metadata or {}).get("operation_class"),
                    "has_object_schema": params_dict.get("type") == "object",
                }
            )
        summary[profile] = entries

    assert [item.name for item in collie_tools(connection)] == [
        item.name for item in collie_tools(connection, profile="browser")
    ]
    rendered = json.dumps(summary, indent=2, sort_keys=True) + "\n"
    if not TOOLS_SNAPSHOT.exists():
        SNAPSHOT_DIR.mkdir(parents=True, exist_ok=True)
        TOOLS_SNAPSHOT.write_text(rendered, encoding="utf-8")
    assert rendered == TOOLS_SNAPSHOT.read_text(encoding="utf-8")


@pytest.mark.covers("adapter:langchain", "story:11-242")
def test_kept_off_operations_and_granting_params_absent() -> None:
    """Offers none of the kept-off operations or granting params on any profile."""
    connection = _StubConnection()
    forbidden = {op.replace(".", "_") for op in KEPT_OFF_OPERATIONS}
    granting_found: list[str] = []

    for profile in ALL_PROFILES:
        tools = collie_tools(connection, profile=profile)
        for item in tools:
            assert item.name not in forbidden, f"{profile} exposed {item.name}"
            assert not item.name.startswith("live_view_"), (
                f"{profile} exposed {item.name}"
            )
            if item.name in {"run_start", "recipe_start"}:
                converted = convert_to_openai_tool(item)
                parameters = converted["function"].get("parameters", {})
                props = parameters.get("properties", {})
                if isinstance(props, dict) and "accept_plugin_change" in props:
                    granting_found.append(f"{profile}:{item.name}")

    assert granting_found == []
    assert "page_lend" in forbidden
    assert len(COLLIE_MCP_PROFILES["browser"]) > 0


@pytest.mark.covers("adapter:langchain", "story:11-242")
def test_announces_tool_form_and_never_retries_retryable_errors() -> None:
    """Announces form `tool`, never retries retryable errors, and returns error JSON."""
    connection = _StubConnection()
    tools = {item.name: item for item in collie_tools(connection, profile="browser")}

    ok_raw = tools["run_start"].invoke({})
    assert json.loads(ok_raw) == {"ok": True, "result": {"run_id": "r1"}}

    err_raw = tools["page_goto"].invoke(
        {"page_id": "p1", "url": "https://example.test"}
    )
    err_payload = json.loads(err_raw)
    assert err_payload["ok"] is False
    assert err_payload["error"]["code"] == "setting_not_applicable"
    assert err_payload["error"]["retryable"] is True
    assert "mcp.oauth.issuer" in err_payload["error"]["fix"]

    async def _run_async() -> str:
        return str(await tools["run_start"].ainvoke({}))

    async_ok = anyio.run(_run_async)
    assert json.loads(async_ok) == {"ok": True, "result": {"run_id": "r1"}}

    assert connection.calls == [
        ("run.start", {}, "tool"),
        ("page.goto", {"page_id": "p1", "url": "https://example.test"}, "tool"),
        ("run.start", {}, "tool"),
    ]


@pytest.mark.covers("adapter:langchain", "story:11-242")
def test_clean_process_import_loads_no_ai_provider_or_playwright() -> None:
    """Importing `goodfetch_collie_langchain` loads no AI provider or Playwright."""
    code = (
        "import json, sys\n"
        "import goodfetch_collie_langchain\n"
        "mods = sorted(sys.modules.keys())\n"
        "print(json.dumps(mods))\n"
    )
    completed = subprocess.run(
        [sys.executable, "-c", code],
        check=True,
        capture_output=True,
        text=True,
    )
    loaded: list[str] = json.loads(completed.stdout)
    forbidden_prefixes = (
        "playwright",
        "openai",
        "anthropic",
        "google.genai",
        "langchain_openai",
        "langchain_anthropic",
        "langchain_google_genai",
        "langchain_aws",
    )
    hits = [
        mod
        for mod in loaded
        if any(
            mod == prefix or mod.startswith(f"{prefix}.")
            for prefix in forbidden_prefixes
        )
    ]
    assert hits == []
    assert "goodfetch_collie_langchain" in loaded
