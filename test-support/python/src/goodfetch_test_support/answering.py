"""A caller of an SDK's generated operations that answers in place of the engine.

An SDK integration test hands it to the operations, as the connection would
be, to build a result from what the engine would send, with no engine running.
"""

from pydantic import BaseModel


class EngineAnswering:
    """A caller answering every call with one result, as the engine sends it."""

    def __init__(self, result: object) -> None:
        """Takes the result every call answers."""
        self._result = result
        self.sent: list[BaseModel] = []
        """The params of each call, in order."""

    def call(self, operation: str, params: BaseModel) -> object:
        """Keeps the params and answers the call with the result."""
        del operation
        self.sent.append(params)
        return self._result
