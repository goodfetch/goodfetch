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


class BindToolsChatModel(GenericFakeChatModel):
    """`GenericFakeChatModel` with the `bind_tools` LangChain's agent loop calls.

    Its turns are the scripted `messages`, as its parent's; an `AIMessage`
    with `tool_calls` asks the loop to call those tools. Each bound copy
    shares the script and refuses a scripted call to a tool it was not
    given, so a misnamed call fails the test instead of reaching the loop as
    a "tool not found" result the model would read.
    """

    bound_tools: tuple[str, ...] | None = None
    """The names of the tools bound to this copy, or `None` before `bind_tools`."""

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
        names = tuple(
            convert_to_openai_tool(each)["function"]["name"] for each in tools
        )
        return self.model_copy(update={"bound_tools": names})

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
        if self.bound_tools is not None and isinstance(message, AIMessage):
            unbound = [
                call["name"]
                for call in message.tool_calls
                if call["name"] not in self.bound_tools
            ]
            if unbound:
                bound = ", ".join(self.bound_tools) or "none"
                text = (
                    f"the script calls {', '.join(unbound)}, "
                    f"which is not bound (bound: {bound})"
                )
                raise ValueError(text)
        return result
