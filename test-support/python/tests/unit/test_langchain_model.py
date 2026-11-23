"""Seam B (spec #326): LangChain's agent loop calls a tool bound to the scripted model.

ADR 0070 drives each Python LangChain tool through the framework's agent loop
with its own scripted model. `GenericFakeChatModel` lacks `bind_tools`, which
the loop calls first; test support's `BindToolsChatModel` adds it.
"""

import pytest
from goodfetch_test_support.langchain_model import (
    BindToolsChatModel,
    UnboundToolCalled,
)
from langchain.agents import (
    create_agent,  # pyright: ignore[reportUnknownVariableType]
)
from langchain.agents.middleware.types import InputAgentState
from langchain_core.language_models import BaseChatModel
from langchain_core.language_models.fake_chat_models import GenericFakeChatModel
from langchain_core.messages import AIMessage, AnyMessage, HumanMessage, ToolMessage
from langchain_core.tools import BaseTool, tool

TITLES = {"https://shop.test/": "Shop", "https://shop.test/cart": "Cart"}
"""The pages the fixture tool knows."""


@tool
def page_title(url: str) -> str:
    """Gives the title of the page at `url`."""
    return TITLES[url]


@tool
def page_count() -> int:
    """Gives how many pages the fixture tool knows; the script never calls it."""
    return len(TITLES)


def _script(url: str) -> list[AIMessage]:
    """The scripted turns: a `page_title` call for `url`, then a final answer."""
    call = {"name": "page_title", "args": {"url": url}, "id": "call-1"}
    return [AIMessage(content="", tool_calls=[call]), AIMessage(content="done")]


def _ask(model: BaseChatModel, tools: list[BaseTool], text: str) -> list[AnyMessage]:
    """Runs LangChain's agent loop on `text` and gives its messages."""
    # LangGraph leaves the agent's context type unknown when it takes none.
    agent = create_agent(model, tools=tools)  # pyright: ignore[reportUnknownVariableType]
    output = agent.invoke(  # pyright: ignore[reportUnknownMemberType]
        InputAgentState(messages=[HumanMessage(content=text)])
    )
    messages: list[AnyMessage] = output["messages"]
    return messages


def test_the_agent_loop_calls_a_bound_tool_and_hands_its_result_back() -> None:
    """The loop calls the scripted tool and reads its result before answering."""
    model = BindToolsChatModel(messages=iter(_script("https://shop.test/cart")))

    messages = _ask(model, [page_title], "Which page is the cart?")

    results = [m for m in messages if isinstance(m, ToolMessage)]
    assert [(m.name, m.content, m.status) for m in results] == [
        ("page_title", "Cart", "success")
    ]
    assert messages[-1].content == "done"


def test_a_tool_that_raises_fails_the_run() -> None:
    """A broken tool's error reaches the test through the loop."""
    model = BindToolsChatModel(messages=iter(_script("https://shop.test/gone")))

    with pytest.raises(KeyError, match=r"https://shop\.test/gone"):
        _ask(model, [page_title], "Which page is gone?")


def test_the_plain_scripted_model_cannot_enter_the_loop() -> None:
    """Why the subclass exists: its parent has no `bind_tools`."""
    model = GenericFakeChatModel(messages=iter(_script("https://shop.test/")))

    with pytest.raises(NotImplementedError):
        _ask(model, [page_title], "Which page is the shop?")


def test_a_scripted_call_to_a_tool_never_bound_is_refused() -> None:
    """A misnamed call fails the test instead of reaching the loop."""
    model = BindToolsChatModel(messages=iter(_script("https://shop.test/")))

    with pytest.raises(UnboundToolCalled) as refused:
        _ask(model, [page_count], "Which page is the shop?")

    assert (refused.value.unbound, refused.value.bound) == (
        ("page_title",),
        ("page_count",),
    )


def test_a_scripted_tool_call_in_a_loop_without_tools_is_refused() -> None:
    """The loop binds nothing when it has no tools, and the call still fails."""
    model = BindToolsChatModel(messages=iter(_script("https://shop.test/")))

    with pytest.raises(UnboundToolCalled) as refused:
        _ask(model, [], "Which page is the shop?")

    assert (refused.value.unbound, refused.value.bound) == (("page_title",), ())
