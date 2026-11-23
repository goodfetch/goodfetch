"""The scripted model that drives Python LangChain tools in tests (ADR 0070).

LangChain's agent loop binds its tools to the model before the first turn.
`GenericFakeChatModel`, LangChain's own scripted model, has no `bind_tools`;
`BindToolsChatModel` adds it, so a test drives a tool through the loop as a
user's agent does, never through the tool's own call alone. It is not
imported with the plugin: a test of a LangChain tool imports it.
"""

from collections.abc import Callable, Sequence
from typing import Any, override

from langchain_core.callbacks import CallbackManagerForLLMRun
from langchain_core.language_models import LanguageModelInput
from langchain_core.language_models.fake_chat_models import GenericFakeChatModel
from langchain_core.messages import AIMessage, BaseMessage
from langchain_core.outputs import ChatResult
from langchain_core.runnables import Runnable
from langchain_core.tools import BaseTool
from langchain_core.utils.function_calling import convert_to_openai_tool
from pydantic import PrivateAttr


class UnboundToolCalled(Exception):
    """A scripted turn calls a tool that is not bound to the model."""

    def __init__(self, unbound: tuple[str, ...], bound: tuple[str, ...]) -> None:
        """Names the tools the turn calls and those bound."""
        super().__init__(
            f"the script calls {', '.join(unbound)}, which is not bound "
            f"(bound: {', '.join(bound) or 'none'})"
        )
        self.unbound = unbound
        """The tools the turn calls that are not bound."""
        self.bound = bound
        """The tools bound to the model."""


class BindToolsChatModel(GenericFakeChatModel):
    """`GenericFakeChatModel` with the `bind_tools` LangChain's agent loop calls.

    Its turns are the scripted `messages`, as its parent's; an `AIMessage`
    with `tool_calls` asks the loop to call those tools. Each bound copy
    shares the script. A scripted call to a tool the model was not given
    raises `UnboundToolCalled`, also when no tool was bound at all (the loop
    binds none when it has none), so a misnamed call fails the test instead
    of reaching the loop as a "tool not found" result or ending the run.
    """

    _bound_tools: tuple[str, ...] = PrivateAttr(default=())

    @override
    def bind_tools(
        self,
        tools: Sequence[dict[str, Any] | type | Callable[..., Any] | BaseTool],
        *,
        tool_choice: str | None = None,
        **kwargs: Any,
    ) -> Runnable[LanguageModelInput, AIMessage]:
        """Gives a copy that answers from the same script with `tools` bound.

        Each tool is converted as a provider's model converts it, so a tool
        whose schema no provider could send fails here.

        Args:
            tools: The tools, in any form LangChain's providers accept.
            tool_choice: Accepted and unused: the script decides each call.
            kwargs: Accepted and unused, as `tool_choice`.
        """
        bound = self.model_copy()
        bound._bound_tools = tuple(
            convert_to_openai_tool(each)["function"]["name"] for each in tools
        )
        return bound

    @override
    def _generate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: CallbackManagerForLLMRun | None = None,
        **kwargs: Any,
    ) -> ChatResult:
        result = super()._generate(messages, stop, run_manager, **kwargs)
        message = result.generations[0].message
        if isinstance(message, AIMessage):
            unbound = tuple(
                call["name"]
                for call in message.tool_calls
                if call["name"] not in self._bound_tools
            )
            if unbound:
                raise UnboundToolCalled(unbound, self._bound_tools)
        return result
