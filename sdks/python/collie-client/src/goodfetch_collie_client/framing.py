"""The Connection's framing (ADRs 0024 and 0038).

One JSON-RPC 2.0 message per line of UTF-8, each way. The engine's side is
read strictly, as `StrictStdout` reads it in Rust's tests (#361): a line that
is not exactly one JSON-RPC message, such as a log line on an Attached
engine's stdout, or that is not UTF-8, is reported as invalid, never
skipped. A partial line at the end is dropped: the engine stopped mid-write,
and its end, with its exit status, says more than the partial line would.
TypeScript's `framing.ts` is its twin.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import TYPE_CHECKING, TypeAlias, cast

if TYPE_CHECKING:
    from collections.abc import Iterator, Mapping

MessageId: TypeAlias = str | int | float
"""A JSON-RPC request or reply id."""


@dataclass(frozen=True)
class Request:
    """A request the engine sent, which waits for the Client's reply."""

    id: MessageId
    method: str
    params: object


@dataclass(frozen=True)
class Notification:
    """A notification the engine sent, which takes no reply."""

    method: str
    params: object


@dataclass(frozen=True)
class Result:
    """The engine's successful reply to the Client's request `id`."""

    id: MessageId | None
    result: object


@dataclass(frozen=True)
class Failure:
    """The engine's error reply to the Client's request `id`."""

    id: MessageId | None
    error: Mapping[str, object]


Message: TypeAlias = Request | Notification | Result | Failure
"""One message the engine sent."""


@dataclass(frozen=True)
class Invalid:
    """A line that is not exactly one JSON-RPC 2.0 message, as received."""

    line: str


_MEMBERS = frozenset({"jsonrpc", "id", "method", "params", "result", "error"})


def _refuse_constant(name: str) -> object:
    # JSON has no NaN or Infinity, which Python's parser takes by default.
    message = f"{name} is no JSON value"
    raise ValueError(message)


def object_members(value: object) -> dict[str, object] | None:
    """`value`'s members when it is a JSON object, else `None`."""
    if not isinstance(value, dict):
        return None
    members = cast("dict[object, object]", value)
    return {str(key): member for key, member in members.items()}


def _is_params(value: object) -> bool:
    return value is None or isinstance(value, dict | list)


def _is_id(value: object) -> bool:
    # A bool is an int to Python, never a number to JSON-RPC.
    return isinstance(value, str | int | float) and not isinstance(value, bool)


def parse_line(line: str) -> Message | None:
    """The message `line` holds, or `None` unless it is one JSON-RPC 2.0 message."""
    if line.strip() != line or not line:
        return None
    try:
        value: object = json.loads(line, parse_constant=_refuse_constant)
    except ValueError:
        return None
    members = object_members(value)
    if members is None:
        return None
    if members.get("jsonrpc") != "2.0" or not members.keys() <= _MEMBERS:
        return None
    message_id = members.get("id")
    method = members.get("method")
    params = members.get("params")
    if "method" in members:
        if not isinstance(method, str) or not _is_params(params):
            return None
        if "result" in members or "error" in members:
            return None
        if "id" not in members:
            return Notification(method, params)
        if not _is_id(message_id):
            return None
        return Request(cast("MessageId", message_id), method, params)
    if not (_is_id(message_id) or message_id is None):
        return None
    reply_id = cast("MessageId | None", message_id)
    if ("result" in members) == ("error" in members):
        return None
    if "result" in members:
        return Result(reply_id, members["result"])
    error = members["error"]
    if not isinstance(error, dict):
        return None
    fields = cast("dict[str, object]", error)
    code = fields.get("code")
    if not _is_id(code) or isinstance(code, str):
        return None
    if not isinstance(fields.get("message"), str):
        return None
    return Failure(reply_id, fields)


def _line_of(data: bytes) -> Message | Invalid:
    try:
        line = data.decode("utf-8")
    except UnicodeDecodeError:
        return Invalid(data.decode("utf-8", errors="replace"))
    message = parse_line(line)
    return Invalid(line) if message is None else message


class LineReader:
    """Splits the engine's bytes into lines, each read as one message.

    Lines are split on bytes, so a character split across chunks stays
    whole, and each is decoded strictly. Only the new chunk is searched for
    a line feed: reading a line costs time linear in its size, however many
    chunks deliver it.
    """

    def __init__(self) -> None:
        """Starts with no partial line."""
        self._partial: list[bytes] = []

    def feed(self, chunk: bytes) -> Iterator[Message | Invalid]:
        """Each whole line `chunk` ends, in order; the rest waits for its line feed."""
        start = 0
        end = chunk.find(b"\n")
        while end != -1:
            if self._partial:
                self._partial.append(chunk[start:end])
                line = b"".join(self._partial)
                self._partial.clear()
            else:
                line = chunk[start:end]
            yield _line_of(line)
            start = end + 1
            end = chunk.find(b"\n", start)
        if start < len(chunk):
            self._partial.append(chunk[start:])


def encode_message(message: Mapping[str, object]) -> bytes:
    """The bytes of one message, its line ended.

    Raises:
        ValueError: The message holds a number JSON cannot carry, such as NaN.
    """
    whole = {"jsonrpc": "2.0", **message}
    line = json.dumps(whole, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
    return (line + "\n").encode()
