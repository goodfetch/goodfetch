"""Mapping the catalog to each language and surface (ADR 0099, decided in #471).

Every name is the catalog's canonical snake_case words: members, parameters and fields keep them; classes are
PascalCase with an initialism from the closed list in capitals (`CDPSession`, `AsyncCDPSession`). A hard keyword used as
an identifier takes a trailing underscore (`route.continue_()`, `locator.and_()`), never in dict keys or literals. A read
the SDK answers itself is a property, except a name starting `is_` or `as_`, which is a method (`response.ok`,
`page.is_closed()`), Playwright for Python's generator rule. Every class with a round trip has an `Async…` twin in the
same module. The base classes `Connection` and `AsyncConnection` sit in `_lifecycle` beside the connection they type.
The sketch's other initialism names (`JsonValue`, `JsonObject`, `JsonSchema`, `UrlMatcher`, `PageApi`, `AiWorkResult`,
`ResponseNotJsonError`) become `JSONValue`, `JSONObject`, `JSONSchema`, `URLMatcher`, `PageAPI`, `AIWorkResult` and
`ResponseNotJSONError` when #483 rewrites the sketch.
"""

from goodfetch._data import JsonObject, JsonValue
from goodfetch._time import Timeout

class AsyncRawCall:
    async def call(self, operation: str, params: JsonObject | None = None, /, *, timeout: Timeout = ...) -> JsonValue:
        """The raw catalog call: the Client layer's way down (G8).

        It calls an operation this SDK does not generate, such as one a newer engine minor added. `params` and the
        result are the wire's snake_case JSON. On a value, `operation` must be in the value's own namespace (`page.…` on a
        page), and the value fills the parameter of that name; any other operation raises a `ProgramError` before it is
        sent. On a connection, `params` names every value itself. A closed value raises `ClosedError`, the timeout and
        task cancellation apply, failures raise the same classes (an unknown code as its group's class), nothing is
        retried, the engine validates `params` strictly, and the Run trace records the call, marked raw. It returns the
        result only, never events.
        """

class RawCall:
    def call(self, operation: str, params: JsonObject | None = None, /, *, timeout: Timeout = ...) -> JsonValue:
        """The raw catalog call (G8), as `AsyncRawCall.call`."""
