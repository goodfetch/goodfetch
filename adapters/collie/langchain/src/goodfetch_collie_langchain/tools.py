"""Collie's LangChain and LangGraph tools for Python (ADRs 0011, 0070, 0095, 0099).

Projects Collie's canonical MCP tool profiles onto `langchain_core.tools.BaseTool`
subclasses usable with both LangChain and LangGraph. Loads no AI provider or
Playwright on import (ADR 0071).
"""

from __future__ import annotations

import inspect
import json
from functools import cache
from typing import TYPE_CHECKING, Any, Protocol, cast, runtime_checkable

import anyio.to_thread
from goodfetch_collie_client.generated.mcp_tools import (
    McpOperationClass,
    McpProfile,
    McpToolSpec,
    collie_mcp_tools,
)
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from langchain_core.tools import BaseTool
from pydantic import BaseModel, ConfigDict, Field, PrivateAttr, create_model

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client.generated.models import Form
    from langchain_core.callbacks import (
        AsyncCallbackManagerForToolRun,
        CallbackManagerForToolRun,
    )
    from pydantic import JsonValue

__all__ = [
    "CollieTool",
    "McpOperationClass",
    "McpProfile",
    "collie_tools",
]


@runtime_checkable
class SyncCollieCaller(Protocol):
    """Sync Collie connection interface accepted by `collie_tools`."""

    def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        form: Form | str | None = None,
    ) -> JsonValue:
        """Sends one raw catalog call synchronously."""
        ...


@runtime_checkable
class AsyncCollieCaller(Protocol):
    """Async Collie connection interface accepted by `collie_tools`."""

    async def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        form: Form | str | None = None,
    ) -> JsonValue:
        """Sends one raw catalog call asynchronously."""
        ...


CollieConnection = SyncCollieCaller | AsyncCollieCaller


def _pascal(name: str) -> str:
    return "".join(part.capitalize() for part in name.split("_") if part)


def _python_type(schema: Mapping[str, Any], *, required: bool) -> object:
    schema_type = schema.get("type")
    if schema_type == "string":
        return str if required else str | None
    if schema_type == "integer":
        return int if required else int | None
    if schema_type == "number":
        return (float | int) if required else (float | int | None)
    if schema_type == "boolean":
        return bool if required else bool | None
    if schema_type == "array":
        return list[Any] if required else list[Any] | None
    if schema_type == "object":
        return dict[str, Any] if required else dict[str, Any] | None
    return Any


@cache
def _input_model_for(tool_name: str, schema_json: str) -> type[BaseModel]:
    schema = cast("dict[str, Any]", json.loads(schema_json))
    raw_props: object = schema.get("properties")
    properties: dict[str, Any] = (
        cast("dict[str, Any]", raw_props) if isinstance(raw_props, dict) else {}
    )
    raw_required: object = schema.get("required")
    required: set[str] = (
        {item for item in cast("list[object]", raw_required) if isinstance(item, str)}
        if isinstance(raw_required, list)
        else set()
    )

    fields: dict[str, Any] = {}
    for prop_name, prop_schema in properties.items():
        prop_map: Mapping[str, Any] = (
            cast("dict[str, Any]", prop_schema) if isinstance(prop_schema, dict) else {}
        )
        is_required = prop_name in required
        annotation = _python_type(prop_map, required=is_required)
        description: object = prop_map.get("description")
        desc = description if isinstance(description, str) else None
        if is_required:
            fields[prop_name] = (annotation, Field(description=desc))
        else:
            fields[prop_name] = (annotation, Field(default=None, description=desc))

    model_config = ConfigDict(extra="allow")
    return create_model(
        f"{_pascal(tool_name)}Input",
        __config__=model_config,
        **fields,
    )


def _format_error(error: GoodfetchError, fallback_operation: str) -> str:
    payload: dict[str, Any] = {
        "code": error.code,
        "group": error.group,
        "retryable": error.retryable,
        "message": str(error),
        "operation": error.operation or fallback_operation,
    }
    if error.fix is not None:
        payload["fix"] = error.fix
    if error.target is not None:
        payload["target"] = error.target
    if error.url is not None:
        payload["url"] = error.url
    if error.observed is not None:
        payload["observed"] = error.observed
    if error.limit is not None:
        payload["limit"] = error.limit
    if error.details:
        payload["details"] = dict(error.details)
    return json.dumps({"ok": False, "error": payload})


def _clean_params(kwargs: Mapping[str, Any]) -> dict[str, JsonValue]:
    return {key: value for key, value in kwargs.items() if value is not None}


class CollieTool(BaseTool):
    """One Collie catalog operation projected as a LangChain `BaseTool`."""

    operation: str
    _connection: Any = PrivateAttr()

    def __init__(
        self,
        *,
        connection: CollieConnection,
        spec: McpToolSpec,
    ) -> None:
        """Builds a LangChain tool backed by `connection` for `spec`."""
        schema_json = json.dumps(spec.input_schema, sort_keys=True)
        model = _input_model_for(spec.name, schema_json)
        super().__init__(
            name=spec.name,
            description=spec.description,
            args_schema=model,
            operation=spec.operation,
            metadata={
                "read_only": spec.read_only,
                "operation_class": spec.operation_class,
                "destructive": spec.destructive,
                "idempotent": spec.idempotent,
                "open_world": spec.open_world,
            },
        )
        self._connection = connection

    def _run(
        self,
        *args: object,
        run_manager: CallbackManagerForToolRun | None = None,
        **kwargs: object,
    ) -> str:
        del args, run_manager
        call_fn = self._connection.call
        if inspect.iscoroutinefunction(call_fn):
            msg = (
                f"{self.name}: cannot invoke synchronously on an async Collie "
                "connection; use `ainvoke` or pass a synchronous `Collie` connection"
            )
            raise TypeError(msg)
        params = _clean_params(kwargs)
        try:
            result = call_fn(self.operation, params, form="tool")
        except GoodfetchError as error:
            return _format_error(error, self.operation)
        return json.dumps({"ok": True, "result": result})

    async def _arun(
        self,
        *args: object,
        run_manager: AsyncCallbackManagerForToolRun | None = None,
        **kwargs: object,
    ) -> str:
        del args, run_manager
        call_fn = self._connection.call
        params = _clean_params(kwargs)
        try:
            if inspect.iscoroutinefunction(call_fn):
                result = await call_fn(self.operation, params, form="tool")
            else:
                result = await anyio.to_thread.run_sync(
                    lambda: call_fn(self.operation, params, form="tool")
                )
        except GoodfetchError as error:
            return _format_error(error, self.operation)
        return json.dumps({"ok": True, "result": result})


def collie_tools(
    connection: CollieConnection,
    *,
    profile: McpProfile = "browser",
) -> list[CollieTool]:
    """Returns LangChain tools for Collie's chosen MCP profile (`browser` by default).

    The returned tools work directly with both LangChain and LangGraph, and
    announce naming form `"tool"` on every call so engine error messages and
    `fix` hints render in tool form (ADRs 0011, 0070, 0095 and 0099).
    """
    return [
        CollieTool(connection=connection, spec=spec)
        for spec in collie_mcp_tools(profile)
    ]
