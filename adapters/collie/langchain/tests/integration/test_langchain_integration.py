"""Integration tests for `goodfetch-collie-langchain` (ADRs 0011, 0070, 0095, 0099).

Covers:
- `ToolsIntegrationTests` from pinned `langchain-tests` on one tool per
  operation class (`read`, `idempotent_write`, `effect`) and every tool a
  Journey uses, against a real Attached Collie engine serving the Proving
  ground.
- A read, an Effect, and a failing call scripted through Python LangChain's
  agent loop (`create_agent`) with `BindToolsChatModel` (`GenericFakeChatModel`
  subclass from `goodfetch_test_support.langchain_model`).
- Running the same tools through LangGraph (`StateGraph` + `ToolNode`).
"""

from __future__ import annotations

import json
from typing import TYPE_CHECKING, Any, cast, override

import pytest
from goodfetch_collie import Collie
from goodfetch_collie_client.generated.mcp_tools import COLLIE_MCP_TOOLS
from goodfetch_collie_langchain import CollieTool, collie_tools
from goodfetch_test_support.langchain_model import BindToolsChatModel
from langchain.agents import (
    create_agent,  # pyright: ignore[reportUnknownVariableType]
)
from langchain_core.messages import AIMessage, BaseMessage, HumanMessage, ToolMessage
from langchain_core.outputs import ChatGeneration, ChatResult
from langchain_core.runnables import RunnableLambda
from langchain_tests.integration_tests import ToolsIntegrationTests
from langgraph.graph import (  # pyright: ignore[reportMissingTypeStubs]
    MessagesState,
    StateGraph,
)
from langgraph.prebuilt import ToolNode, tools_condition
from pydantic import PrivateAttr

if TYPE_CHECKING:
    from collections.abc import Callable, Generator, Mapping

    from goodfetch_test_support import ProvingGround
    from langchain_core.callbacks import CallbackManagerForLLMRun


@pytest.fixture
def proving_ground_seed() -> int:
    """Seed for the local Proving ground."""
    return 2403


@pytest.fixture
def with_browser() -> bool:
    """Enables the Browser cache for Collie's Attached engine."""
    return True


def _parse_last_tool_payload(
    messages: list[BaseMessage],
) -> dict[str, Any] | None:
    for message in reversed(messages):
        if isinstance(message, ToolMessage) and isinstance(message.content, str):
            parsed: object = json.loads(message.content)
            if isinstance(parsed, dict):
                return cast("dict[str, Any]", parsed)
    return None


def _require_result_str(payload: dict[str, Any] | None, key: str) -> str:
    if payload is None:
        msg = f"expected tool payload with result.{key}"
        raise TypeError(msg)
    raw_result: object = payload.get("result")
    if not isinstance(raw_result, dict):
        msg = f"expected string field result.{key}"
        raise TypeError(msg)
    result = cast("dict[str, Any]", raw_result)
    val: object = result.get(key)
    if not isinstance(val, str):
        msg = f"expected string field result.{key}"
        raise TypeError(msg)
    return val


class _DynamicBindToolsChatModel(BindToolsChatModel):
    """`BindToolsChatModel` driven by a stateful step function over tool outputs."""

    _decide: Callable[[list[BaseMessage]], AIMessage] = PrivateAttr()

    def __init__(self, decide: Callable[[list[BaseMessage]], AIMessage]) -> None:
        super().__init__(messages=iter(()))
        self._decide = decide

    @override
    def _generate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: CallbackManagerForLLMRun | None = None,
        **kwargs: object,
    ) -> ChatResult:
        del stop, run_manager, kwargs
        message = self._decide(messages)
        unbound = tuple(
            call["name"]
            for call in message.tool_calls
            if call["name"] not in self._bound_tools
        )
        if unbound:
            from goodfetch_test_support.langchain_model import UnboundToolCalled

            raise UnboundToolCalled(unbound, self._bound_tools)
        return ChatResult(generations=[ChatGeneration(message=message)])


@pytest.mark.covers("adapter:langchain", "story:11-242")
def test_langchain_agent_scripts_read_effect_and_failing_call(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Scripts a read, an Effect, and a failing call through Python LangChain."""
    product_url = proving_ground.start_url("catalog/2")
    with Collie.connect(env=env) as collie:
        tools = collie_tools(collie, profile="browser")
        state: dict[str, Any] = {
            "step": 0,
            "run_id": "",
            "page_id": "",
            "record": {},
            "error_payload": None,
        }

        def decide(messages: list[BaseMessage]) -> AIMessage:
            state["step"] += 1
            step = state["step"]
            payload = _parse_last_tool_payload(messages)
            if step == 1:
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "p1",
                            "name": "run_start",
                            "args": {},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 2:
                state["run_id"] = _require_result_str(payload, "run_id")
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "p2",
                            "name": "run_new_session",
                            "args": {"run_id": state["run_id"]},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 3:
                session_id = _require_result_str(payload, "session_id")
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "p3",
                            "name": "session_new_page",
                            "args": {"session_id": session_id},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 4:
                state["page_id"] = _require_result_str(payload, "page_id")
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "p4",
                            "name": "page_goto",
                            "args": {"page_id": state["page_id"], "url": product_url},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 5:
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "p5",
                            "name": "page_read",
                            "args": {
                                "page_id": state["page_id"],
                                "fields": {
                                    "id": {
                                        "locator": {
                                            "kind": "css",
                                            "selector": "span.id",
                                        }
                                    },
                                    "name": {
                                        "locator": {
                                            "kind": "css",
                                            "selector": "h1.name",
                                        }
                                    },
                                    "price": {
                                        "locator": {
                                            "kind": "css",
                                            "selector": "span.price",
                                        },
                                        "attribute": "data-price",
                                        "transform": {
                                            "kind": "number",
                                            "locale": "en",
                                        },
                                    },
                                },
                            },
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 6:
                if payload and isinstance(payload.get("result"), dict):
                    values = payload["result"].get("values")
                    if isinstance(values, dict):
                        state["record"] = values
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "p6",
                            "name": "run_start",
                            "args": {
                                "settings": {"mcp.oauth.issuer": "https://example.test"}
                            },
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 7:
                state["error_payload"] = payload
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "p7",
                            "name": "run_finish",
                            "args": {"run_id": state["run_id"]},
                            "type": "tool_call",
                        }
                    ],
                )
            return AIMessage(content="python-langchain-done")

        model = _DynamicBindToolsChatModel(decide)
        agent = create_agent(  # pyright: ignore[reportUnknownVariableType]
            model=model, tools=tools
        )
        outcome = agent.invoke(  # pyright: ignore[reportUnknownMemberType]
            {
                "messages": [
                    HumanMessage(content="Navigate, read, handle error, finish.")
                ]
            }
        )

    answer = proving_ground.judge(
        {"task": "catalog/2", "status": "success", "records": [state["record"]]}
    )
    assert answer.outcome == "verified"
    assert outcome["messages"][-1].content == "python-langchain-done"
    err = state["error_payload"]
    assert isinstance(err, dict)
    assert err["ok"] is False
    error_info = cast("dict[str, Any]", err["error"])
    assert error_info["code"] == "setting_not_applicable"
    assert "mcp.oauth.issuer" in str(error_info["message"])
    assert str(error_info["fix"]).strip()


@pytest.mark.covers("adapter:langchain", "story:11-242")
def test_langgraph_runs_same_collie_tools(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Runs the same `collie_tools` through LangGraph `StateGraph` + `ToolNode`."""
    start_url = proving_ground.start_url("statuses/1")
    with Collie.connect(env=env) as collie:
        tools = collie_tools(collie, profile="browser")
        state: dict[str, Any] = {
            "step": 0,
            "run_id": "",
            "page_id": "",
            "observed_url": "",
        }

        def decide(messages: list[BaseMessage]) -> AIMessage:
            state["step"] += 1
            step = state["step"]
            payload = _parse_last_tool_payload(messages)
            if step == 1:
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "g1",
                            "name": "run_start",
                            "args": {},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 2:
                state["run_id"] = _require_result_str(payload, "run_id")
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "g2",
                            "name": "run_new_session",
                            "args": {"run_id": state["run_id"]},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 3:
                session_id = _require_result_str(payload, "session_id")
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "g3",
                            "name": "session_new_page",
                            "args": {"session_id": session_id},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 4:
                state["page_id"] = _require_result_str(payload, "page_id")
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "g4",
                            "name": "page_goto",
                            "args": {"page_id": state["page_id"], "url": start_url},
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 5:
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "g5",
                            "name": "page_formats",
                            "args": {
                                "page_id": state["page_id"],
                                "formats": {"text": True},
                            },
                            "type": "tool_call",
                        }
                    ],
                )
            if step == 6:
                state["observed_url"] = _require_result_str(payload, "url")
                return AIMessage(
                    content="",
                    tool_calls=[
                        {
                            "id": "g6",
                            "name": "run_finish",
                            "args": {"run_id": state["run_id"]},
                            "type": "tool_call",
                        }
                    ],
                )
            return AIMessage(content="python-langgraph-done")

        bound_model = _DynamicBindToolsChatModel(decide).bind_tools(tools)

        def call_model(graph_state: MessagesState) -> dict[str, Any]:
            response = bound_model.invoke(graph_state["messages"])
            return {"messages": [response]}

        builder: StateGraph[MessagesState, None, MessagesState, MessagesState] = (
            StateGraph(MessagesState)
        )
        builder.add_node(  # pyright: ignore[reportUnknownMemberType]
            "agent", RunnableLambda(call_model)
        )
        builder.add_node(  # pyright: ignore[reportUnknownMemberType]
            "tools", ToolNode(tools)
        )
        builder.add_edge("__start__", "agent")
        builder.add_conditional_edges("agent", tools_condition)
        builder.add_edge("tools", "agent")
        graph = builder.compile()  # pyright: ignore[reportUnknownMemberType]

        outcome = graph.invoke(  # pyright: ignore[reportUnknownMemberType]
            {"messages": [HumanMessage(content="Visit start URL and finish.")]}
        )

    assert outcome["messages"][-1].content == "python-langgraph-done"
    assert state["observed_url"] == start_url


# `ToolsIntegrationTests` on one tool per operation class (`read`: engine_info,
# `idempotent_write`: cache_clear, `effect`: page_goto) plus every tool a
# Collie Journey uses (`run_start`, `run_new_session`, `session_new_page`,
# `page_goto`, `page_read`, `page_formats`, `run_finish`).
@pytest.mark.asyncio
class _LiveCollieToolIntegrationBase(ToolsIntegrationTests):
    """Shared base running `ToolsIntegrationTests` against a real Attached engine."""

    _tool_name: str = ""

    @pytest.fixture
    def live_context(
        self,
        env: Mapping[str, str],
        proving_ground: ProvingGround,
    ) -> Generator[dict[str, Any], None, None]:
        """Opens a live Attached Collie engine and page on the Proving ground."""
        start_url = proving_ground.start_url("catalog/2")
        with Collie.connect(env=env) as collie:
            run_res = collie.call("run.start", {}, form="tool")
            assert isinstance(run_res, dict)
            run_id = str(run_res["run_id"])
            sess_res = collie.call("run.new_session", {"run_id": run_id}, form="tool")
            assert isinstance(sess_res, dict)
            session_id = str(sess_res["session_id"])
            page_res = collie.call(
                "session.new_page", {"session_id": session_id}, form="tool"
            )
            assert isinstance(page_res, dict)
            page_id = str(page_res["page_id"])
            collie.call(
                "page.goto", {"page_id": page_id, "url": start_url}, form="tool"
            )
            yield {
                "collie": collie,
                "run_id": run_id,
                "session_id": session_id,
                "page_id": page_id,
                "start_url": start_url,
            }
            collie.call("run.finish", {"run_id": run_id}, form="tool")

    @pytest.fixture
    def tool(self, live_context: dict[str, Any]) -> CollieTool:
        """Builds the CollieTool under test backed by the live engine."""
        self._live_context = live_context
        return CollieTool(
            connection=live_context["collie"],
            spec=COLLIE_MCP_TOOLS[self._tool_name],
        )

    @property
    def tool_constructor(self) -> type[CollieTool]:
        """Tool class constructor."""
        return CollieTool

    @property
    def tool_constructor_params(self) -> dict[str, Any]:
        """Constructor kwargs for the live tool."""
        return {
            "connection": self._live_context["collie"],
            "spec": COLLIE_MCP_TOOLS[self._tool_name],
        }


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationEngineInfoRead(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for a `read` tool (`engine_info`)."""

    _tool_name = "engine_info"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `engine_info`."""
        return {}


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationCacheClearIdempotentWrite(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for an `idempotent_write` tool (`cache_clear`)."""

    _tool_name = "cache_clear"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `cache_clear`."""
        return {}


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationPageGotoEffect(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for an `effect` + Journey tool (`page_goto`)."""

    _tool_name = "page_goto"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `page_goto`."""
        return {
            "page_id": self._live_context["page_id"],
            "url": self._live_context["start_url"],
        }


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationPageReadJourney(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for Journey tool `page_read`."""

    _tool_name = "page_read"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `page_read`."""
        return {
            "page_id": self._live_context["page_id"],
            "fields": {"name": {"locator": {"kind": "css", "selector": "h1.name"}}},
        }


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationPageFormatsJourney(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for Journey tool `page_formats`."""

    _tool_name = "page_formats"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `page_formats`."""
        return {
            "page_id": self._live_context["page_id"],
            "formats": {"text": True},
        }


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationRunStartJourney(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for Journey tool `run_start`."""

    _tool_name = "run_start"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `run_start`."""
        return {}


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationRunNewSessionJourney(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for Journey tool `run_new_session`."""

    _tool_name = "run_new_session"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `run_new_session`."""
        return {"run_id": self._live_context["run_id"]}


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationSessionNewPageJourney(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for Journey tool `session_new_page`."""

    _tool_name = "session_new_page"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `session_new_page`."""
        return {"session_id": self._live_context["session_id"]}


@pytest.mark.covers("adapter:langchain", "story:11-242")
class TestIntegrationRunFinishJourney(_LiveCollieToolIntegrationBase):
    """`ToolsIntegrationTests` for Journey tool `run_finish`."""

    _tool_name = "run_finish"

    @property
    def tool_invoke_params_example(self) -> dict[str, Any]:
        """Example params for `run_finish`."""
        return {"run_id": self._live_context["run_id"]}
