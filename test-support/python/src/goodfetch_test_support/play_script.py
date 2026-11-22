"""The Scripted engine's in-process player (ADR 0069).

Plays one script of the shared corpus, `test-support/fixtures/engine-scripts/`,
to a Client over an in-memory transport, as `goodfetch-scripted-engine` plays
it over stdio, so an SDK's unit test runs in the Quick suite with no process.
It matches each message the Client sends on its method and params, compared
as their canonical JSON (RFC 8785) would be, answers with the ID of the
request the step before matched, and fails on a message no step expects and
on a step left unused. TypeScript's `playScript` is its twin.
"""

from __future__ import annotations

import codecs
import json
import math
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Literal, TypeAlias, assert_never, cast

import anyio
import anyio.lowlevel
from anyio.abc import ByteStream

from goodfetch_test_support.schemas import SchemaError, validate

CORPUS = Path(__file__).resolve().parents[3] / "fixtures" / "engine-scripts"
"""`test-support/fixtures/engine-scripts/`, beside this package's folder."""


@dataclass(frozen=True)
class Finished:
    """The Client closed its end after the last step, or a `stall`."""


@dataclass(frozen=True)
class Closed:
    """The script closed the connection."""


@dataclass(frozen=True)
class Exited:
    """The script's engine exited at once with `code`, as a crash does."""

    code: int


Ending: TypeAlias = Finished | Closed | Exited
"""How a script that played to its end ended."""

Side: TypeAlias = Literal["client", "engine"]
"""Who wrote a message: the Client or the engine."""


@dataclass(frozen=True)
class PlayedMessage:
    """A JSON line the play read or wrote, as `goodfetch-scripted-engine` records it."""

    from_: Side
    message: object


@dataclass(frozen=True)
class PlayedRaw:
    """A line that is not JSON, or a `raw` step's text."""

    from_: Side
    raw: str


Played: TypeAlias = PlayedMessage | PlayedRaw
"""A message the play read or wrote."""


class PlayFailed(Exception):
    """The play failed: the Client's next read and write raise it, as `done` does."""


class UnexpectedMessage(PlayFailed):
    """The Client sent a message no step of the script expects."""


class UnusedStep(PlayFailed):
    """The Client closed its end, or the script ended, with a step unused."""


class NothingToAnswer(PlayFailed):
    """A `reply` or `await_reply` step follows no request to answer."""


class DropUnsupported(PlayFailed):
    """A `drop` step, which only the executable's `websocket` mode plays."""


class _Absent(Enum):
    """The params of a message that carries none, unlike JSON's `null`."""

    ABSENT = "absent"


_ABSENT = _Absent.ABSENT


@dataclass(frozen=True)
class _Call:
    method: str
    params: object | _Absent


@dataclass(frozen=True)
class _Answer:
    """A reply's one member, `result` or `error`, and its value."""

    key: Literal["result", "error"]
    value: object


@dataclass(frozen=True)
class _Expect:
    call: _Call


@dataclass(frozen=True)
class _Reply:
    answer: _Answer


@dataclass(frozen=True)
class _SendRequest:
    call: _Call


@dataclass(frozen=True)
class _SendNotification:
    call: _Call


@dataclass(frozen=True)
class _AwaitReply:
    answer: _Answer


@dataclass(frozen=True)
class _Stall:
    pass


@dataclass(frozen=True)
class _Close:
    pass


@dataclass(frozen=True)
class _Drop:
    pass


@dataclass(frozen=True)
class _Raw:
    text: str


@dataclass(frozen=True)
class _Exit:
    code: int


_Step: TypeAlias = (
    _Expect
    | _Reply
    | _SendRequest
    | _SendNotification
    | _AwaitReply
    | _Stall
    | _Close
    | _Drop
    | _Raw
    | _Exit
)
"""One step of a script; `script.schema.json` is the contract."""


_Id: TypeAlias = str | int | float
"""A JSON-RPC request's ID: a string or a number."""


def play_script(script: str | Path) -> PlayedScript:
    """Plays a script to the Client that takes the returned `transport`.

    Args:
        script: A script of the corpus by its path in it, such as
            `"self-test/call-answered.json"`, or a script file's path.

    Returns:
        The script being played.

    Raises:
        SchemaError: The file fails the committed `script.schema.json`.
    """
    path = CORPUS / script if isinstance(script, str) else script
    value: object = json.loads(path.read_text("utf-8"))
    try:
        validate(value, CORPUS / "script.schema.json")
    except SchemaError as error:
        message = f"the script {path} fails script.schema.json: {error}"
        raise SchemaError(message) from error
    steps = cast("list[dict[str, object]]", cast("dict[str, object]", value)["steps"])
    player = _Player([_step(step) for step in steps])
    return PlayedScript(player, InMemoryTransport(player))


class PlayedScript:
    """A script being played: the transport to hand the SDK, and what it saw."""

    def __init__(self, player: _Player, transport: InMemoryTransport) -> None:
        """Wraps a play that started as `play_script` built it."""
        self._player = player
        self.transport: ByteStream = transport
        """The transport to hand the SDK."""

    @property
    def messages(self) -> list[Played]:
        """Every message read and written so far, in order."""
        return list(self._player.messages)

    def done(self) -> Ending:
        """Ends the play as the Client closing its end would, and says how it ended.

        Returns:
            How the script ended, unless the play failed.

        Raises:
            UnexpectedMessage: The Client sent a message no step expects.
            UnusedStep: A step was left unused.
            NothingToAnswer: A `reply` or `await_reply` follows no request.
        """
        self._player.client_closed()
        return self._player.ending()


class InMemoryTransport(ByteStream):
    """The value an SDK takes as its connection to the engine.

    The engine's bytes to receive, and the Client's bytes to send,
    newline-delimited JSON-RPC each way as on an Attached engine's stdio.
    `send_eof` closes the Client's end.
    """

    def __init__(self, player: _Player) -> None:
        """Joins the transport to its play."""
        self._player = player
        self._pending = b""

    async def receive(self, max_bytes: int = 65536) -> bytes:
        """The engine's next bytes.

        Raises:
            EndOfStream: The script closed the engine's end.
            PlayFailed: The play failed.
        """
        self._player.raise_failure()
        if not self._pending:
            try:
                self._pending = await self._player.engine.receive()
            except anyio.EndOfStream:
                self._player.raise_failure()
                raise
        data, self._pending = self._pending[:max_bytes], self._pending[max_bytes:]
        return data

    async def send(self, item: bytes) -> None:
        """Writes the Client's bytes.

        Raises:
            PlayFailed: The play failed, on these bytes or before them.
        """
        await anyio.lowlevel.checkpoint()
        self._player.take(item)
        self._player.raise_failure()

    async def send_eof(self) -> None:
        """Closes the Client's end.

        Raises:
            PlayFailed: The play failed, such as on a step left unused.
        """
        await anyio.lowlevel.checkpoint()
        self._player.client_closed()
        self._player.raise_failure()

    async def aclose(self) -> None:
        """Closes both ends, as the Client giving up does."""
        self._player.client_closed()
        await self._player.engine.aclose()


@dataclass(frozen=True)
class _Parsed:
    """A line's JSON value, which may be `null`."""

    value: object


class _Player:
    """The state of one play: the step it is at, and what it answers next."""

    def __init__(self, steps: list[_Step]) -> None:
        """Starts the play, writing what the script sends before it first waits."""
        self.messages: list[Played] = []
        self._steps = steps
        self._writer, self.engine = anyio.create_memory_object_stream[bytes](math.inf)
        self._decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self._buffered = ""
        self._index = 0
        # The ID of the request the last `expect` matched, which `reply` answers.
        self._answering: _Id | None = None
        # The ID of the request the last `send` sent, which `await_reply` matches.
        self._awaiting: int | None = None
        self._next_id = 1
        self._outcome: Ending | PlayFailed | None = None
        self._advance()

    def ending(self) -> Ending:
        """How the play ended, once the Client closed its end."""
        outcome = self._outcome
        if isinstance(outcome, PlayFailed):
            raise outcome
        if outcome is None:
            message = "invariant: a closed play has an outcome"
            raise AssertionError(message)
        return outcome

    def raise_failure(self) -> None:
        """Raises the play's failure, if it failed."""
        if isinstance(self._outcome, PlayFailed):
            raise self._outcome

    def take(self, data: bytes) -> None:
        """Takes bytes the Client wrote, reading each whole line."""
        self._buffered += self._decoder.decode(data)
        line, newline, rest = self._buffered.partition("\n")
        while newline:
            self._buffered = rest
            self._read(line)
            line, newline, rest = self._buffered.partition("\n")

    def client_closed(self) -> None:
        """The Client closed its end: after a `stall` or the last step, it ends."""
        if self._outcome is not None:
            return
        self._buffered += self._decoder.decode(b"", final=True)
        if self._buffered:
            line, self._buffered = self._buffered, ""
            self._read(line)
            if self._outcome is not None:
                return
        if self._index < len(self._steps) and isinstance(
            self._steps[self._index], _Stall
        ):
            self._index += 1
        if self._index < len(self._steps):
            self._fail(_unused_step(self._index))
            return
        self._end(Finished())

    def _advance(self) -> None:
        """Plays the steps the engine starts, up to one that waits for the Client."""
        while self._outcome is None and self._index < len(self._steps):
            index = self._index
            step = self._steps[index]
            # As the executable does, an `await_reply` with no request sent
            # before it fails at once.
            if isinstance(step, _AwaitReply) and self._awaiting is None:
                self._fail(_nothing_to_answer(index))
                return
            if isinstance(step, _Expect | _AwaitReply | _Stall):
                return
            self._index += 1
            match step:
                case _Reply(answer):
                    answering, self._answering = self._answering, None
                    if answering is None:
                        self._fail(_nothing_to_answer(index))
                        return
                    self._write(
                        {"jsonrpc": "2.0", "id": answering, answer.key: answer.value}
                    )
                case _SendRequest(call):
                    request_id = self._next_id
                    self._next_id += 1
                    self._write({"jsonrpc": "2.0", "id": request_id, **_members(call)})
                    self._awaiting = request_id
                case _SendNotification(call):
                    self._write({"jsonrpc": "2.0", **_members(call)})
                case _Raw(text):
                    self._writer.send_nowait(text.encode())
                    self.messages.append(PlayedRaw(from_="engine", raw=text))
                case _Close():
                    self._end(Closed())
                case _Drop():
                    # An in-process play has one Client: no other connects.
                    self._fail(_drop_unsupported(index))
                    return
                case _Exit(code):
                    self._end(Exited(code))
                case _:
                    assert_never(step)

    def _read(self, line: str) -> None:
        """Takes one line the Client wrote."""
        if self._outcome is not None:
            return
        parsed = _parse(line)
        self.messages.append(
            PlayedRaw(from_="client", raw=line)
            if parsed is None
            else PlayedMessage(from_="client", message=parsed.value)
        )
        # The real engine refuses what is not one JSON-RPC 2.0 message: a
        # missing or other `jsonrpc`, a reply with both `result` and `error`.
        message = (
            cast("dict[str, object]", parsed.value)
            if parsed is not None and _is_json_rpc(parsed.value)
            else None
        )
        index = self._index
        step = self._steps[index] if index < len(self._steps) else None
        if step is None or isinstance(step, _Stall):
            expected = (
                "the Client to close its end after the last step"
                if step is None
                else "no message until the Client gives up"
            )
            self._fail(_unexpected(None, expected, message, line))
            return
        if message is None:
            self._fail(_unexpected(index, "a JSON-RPC 2.0 message", message, line))
            return
        match step:
            case _Expect(wanted):
                matches = (
                    "result" not in message
                    and "error" not in message
                    and message["method"] == wanted.method
                    and _same(message.get("params", _ABSENT), wanted.params)
                )
                if not matches:
                    expected = f"`{wanted.method}` with params {_text(wanted.params)}"
                    self._fail(_unexpected(index, expected, message, line))
                    return
                # A notification, such as a `$/cancel`, leaves the request
                # matched before it to answer.
                if "id" in message:
                    self._answering = cast("_Id", message["id"])
            case _AwaitReply(answer):
                request_id, self._awaiting = self._awaiting, None
                matches = (
                    "method" not in message
                    and _same(message["id"], request_id)
                    and _same(message.get(answer.key, _ABSENT), answer.value)
                )
                if not matches:
                    expected = (
                        f'the answer {{"{answer.key}": {_text(answer.value)}}} '
                        f"to request {request_id}"
                    )
                    self._fail(_unexpected(index, expected, message, line))
                    return
            case _:
                invariant = (
                    "invariant: the play waits for the Client only at expect, "
                    "await_reply or stall"
                )
                raise AssertionError(invariant)
        self._index += 1
        self._advance()

    def _write(self, message: dict[str, object]) -> None:
        line = json.dumps(message, ensure_ascii=False, separators=(",", ":"))
        self._writer.send_nowait(f"{line}\n".encode())
        self.messages.append(PlayedMessage(from_="engine", message=message))

    def _end(self, ending: Ending) -> None:
        """Ends the play and the engine's end; a step after the ending is unused."""
        if self._index < len(self._steps):
            self._fail(_unused_step(self._index))
            return
        self._outcome = ending
        self._writer.close()

    def _fail(self, failure: PlayFailed) -> None:
        """Fails the play: the next read, write and `done` raise `failure`."""
        self._outcome = failure
        self._writer.close()


def _parse(line: str) -> _Parsed | None:
    """A line's JSON value, or `None` when it is not JSON."""
    try:
        return _Parsed(json.loads(line, parse_constant=_refuse_constant))
    except ValueError:
        return None


def _refuse_constant(name: str) -> object:
    """Refuses `NaN` and `Infinity`, which JSON does not have."""
    message = f"{name} is not JSON"
    raise ValueError(message)


def _is_id(value: object) -> bool:
    """Whether `value` is a JSON-RPC request's ID: a string or a number."""
    return isinstance(value, str | int | float) and not isinstance(value, bool)


def _is_integer(value: object) -> bool:
    """Whether `value` is a JSON number with no fraction, as `Number.isInteger` says."""
    if isinstance(value, bool):
        return False
    return isinstance(value, int) or (isinstance(value, float) and value.is_integer())


def _is_json_rpc(value: object) -> bool:
    """Whether `value` is one JSON-RPC 2.0 message, as the executable judges it.

    A request or notification (`method`, an optional string or number `id`,
    object or array `params`) or a reply (`id` and exactly one of `result`
    and an `error` object), with no other member.
    """
    if not isinstance(value, dict):
        return False
    message = cast("dict[str, object]", value)
    if message.get("jsonrpc") != "2.0":
        return False
    if "method" in message:
        return (
            isinstance(message["method"], str)
            and ("id" not in message or _is_id(message["id"]))
            and ("params" not in message or isinstance(message["params"], dict | list))
            and message.keys() <= {"jsonrpc", "method", "id", "params"}
        )
    if "result" in message:
        outcome = "error" not in message
    else:
        error = message.get("error")
        outcome = isinstance(error, dict) and _is_error(
            cast("dict[str, object]", error)
        )
    return (
        "id" in message
        and (_is_id(message["id"]) or message["id"] is None)
        and outcome
        and message.keys() <= {"jsonrpc", "id", "result", "error"}
    )


def _is_error(error: dict[str, object]) -> bool:
    """Whether `error` is a JSON-RPC error object."""
    return (
        _is_integer(error.get("code"))
        and isinstance(error.get("message"), str)
        and error.keys() <= {"code", "message", "data"}
    )


def _same(left: object, right: object) -> bool:
    """Whether two JSON values, or `absent`, have the same canonical JSON (RFC 8785).

    Python's `==` counts `True == 1`; JSON's `true` is no number. Numbers
    compare as the doubles RFC 8785 writes, so `1` and `1.0` are the same.
    """
    same_type = type(left) is type(right)
    if isinstance(left, bool) or isinstance(right, bool):
        return same_type and left == right
    if isinstance(left, int | float) and isinstance(right, int | float):
        return _double(left) == _double(right)
    if isinstance(left, dict) and isinstance(right, dict):
        left_members = cast("dict[str, object]", left)
        right_members = cast("dict[str, object]", right)
        return left_members.keys() == right_members.keys() and all(
            _same(item, right_members[key]) for key, item in left_members.items()
        )
    if isinstance(left, list) and isinstance(right, list):
        left_items = cast("list[object]", left)
        right_items = cast("list[object]", right)
        return len(left_items) == len(right_items) and all(
            _same(item, other)
            for item, other in zip(left_items, right_items, strict=True)
        )
    # Strings, `null` and `absent`, each equal only to itself.
    return same_type and left == right


def _double(number: float) -> float:
    """A JSON number as JavaScript reads it: the nearest double, or an infinity."""
    try:
        return float(number)
    except OverflowError:
        return math.inf if number > 0 else -math.inf


def _text(value: object) -> str:
    """A value as a failure's message shows it: JSON with sorted keys, or `absent`."""
    if value is _ABSENT:
        return "absent"
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def _unexpected(
    step: int | None, expected: str, message: dict[str, object] | None, line: str
) -> UnexpectedMessage:
    received = (
        json.dumps(line, ensure_ascii=False) if message is None else _text(message)
    )
    where = "the script expects" if step is None else f"step {step} expects"
    return UnexpectedMessage(
        f"unexpected_message: {where} {expected}, and the Client sent {received}"
    )


def _unused_step(step: int) -> UnusedStep:
    return UnusedStep(f"unused_step: step {step} was left unused")


def _nothing_to_answer(step: int) -> NothingToAnswer:
    return NothingToAnswer(
        f"nothing_to_answer: step {step} answers a request, and none came before it"
    )


def _drop_unsupported(step: int) -> DropUnsupported:
    return DropUnsupported(
        f"drop_unsupported: step {step} drops the Client, and this mode takes no other"
    )


def _members(call: _Call) -> dict[str, object]:
    """A call's `method`, and its `params` unless it has none."""
    if call.params is _ABSENT:
        return {"method": call.method}
    return {"method": call.method, "params": call.params}


def _call(value: object) -> _Call:
    call = cast("dict[str, object]", value)
    return _Call(method=cast("str", call["method"]), params=call.get("params", _ABSENT))


def _answer(value: object) -> _Answer:
    ((key, body),) = cast("dict[Literal['result', 'error'], object]", value).items()
    return _Answer(key=key, value=body)


def _step(value: dict[str, object]) -> _Step:
    """Reads one step that `script.schema.json` accepted."""
    ((kind, body),) = value.items()
    fields = cast("dict[str, object]", body)
    match kind:
        case "expect":
            return _Expect(_call(fields))
        case "reply":
            return _Reply(_answer(fields))
        case "send":
            ((sent, call),) = fields.items()
            return (
                _SendRequest(_call(call))
                if sent == "request"
                else _SendNotification(_call(call))
            )
        case "await_reply":
            return _AwaitReply(_answer(fields))
        case "stall":
            return _Stall()
        case "close":
            return _Close()
        case "drop":
            return _Drop()
        case "raw":
            return _Raw(cast("str", fields["text"]))
        case "exit":
            return _Exit(cast("int", fields["code"]))
        case _:
            message = f"invariant: script.schema.json defines no {kind} step"
            raise AssertionError(message)


__all__ = [
    "CORPUS",
    "Closed",
    "DropUnsupported",
    "Ending",
    "Exited",
    "Finished",
    "InMemoryTransport",
    "NothingToAnswer",
    "PlayFailed",
    "Played",
    "PlayedMessage",
    "PlayedRaw",
    "PlayedScript",
    "Side",
    "UnexpectedMessage",
    "UnusedStep",
    "play_script",
]
