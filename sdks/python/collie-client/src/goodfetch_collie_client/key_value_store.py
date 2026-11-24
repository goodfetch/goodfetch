"""A Run's Key-value store (ADR 0103): `run.key_value_store`.

JSON values, and bytes with their content type, by key, kept in the Run's
run state, so a Run resumed by its ID, here or on another machine, finds
them. A `None` from `get` always means an absent key, since `set` takes no
top-level null: an untyped caller's `None` fails in the engine with
`value_invalid`, naming `delete`. One method never answers two types (S2):
`get` on bytes fails with `value_invalid`, and `get_bytes` on JSON. A value
may be typed by a schema in the program's own types (ADR 0082): `set` sends
the form the schema parses back, and `get` answers what it parses. Only
JSON crosses: `set` refuses a value that is not JSON before sending
anything, naming its path.

Its committed keys are listed by `keys(prefix=…)`, an iterator (ADR 0081)
read with `async for`, or `for` in the sync API, in the order of their
Unicode scalar values. The call takes no writer lock and goes when `keys`
is called. A crawl's handler reaches its attempt's own store,
`ctx.run.key_value_store`, whose calls name the attempt, so its writes are
staged with it. TypeScript's `key-value-store.ts` is its twin.
"""

from __future__ import annotations

import base64
import json
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Protocol, TypeAlias, TypeVar, cast, overload

from pydantic import BaseModel

from goodfetch_collie_client._framing import object_members
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.errors import ValueInvalidError
from goodfetch_collie_client.generated.models import (
    KeyValueStoreBytesValue,
    KeyValueStoreContent,
    KeyValueStoreDeleted,
    KeyValueStoreKeyParams,
    KeyValueStoreSetBytesParams,
    KeyValueStoreSetParams,
    KeyValueStoreValue,
)
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)
from goodfetch_collie_client.json_values import JSONValue, non_json
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.schema import (
    SchemaCodec,
    SchemaRejectedError,
    ValueNotJSONError,
    schema_codec,
)

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping

    from typing_extensions import TypeForm

    from goodfetch_collie_client.generated.models import KeyValueStoreAttempt, RunId
    from goodfetch_collie_client.schema import JSONSchema
    from goodfetch_collie_client.sequence import AsyncSequence, Sequence

__all__ = ["AsyncKeyValueStore", "KeyValueStore", "StoredBytes"]

T = TypeVar("T")

StoredValue: TypeAlias = (
    list[JSONValue] | dict[str, JSONValue] | str | bool | int | float
)
"""What the store keeps: any JSON value but a top-level null. A nested null is kept."""

GET = "key_value_store.get"
"""The operation `get` sends."""

SET = "key_value_store.set"
"""The operation `set` sends."""

GET_BYTES = "key_value_store.get_bytes"
"""The operation `get_bytes` sends."""

SET_BYTES = "key_value_store.set_bytes"
"""The operation `set_bytes` sends."""

DELETE = "key_value_store.delete"
"""The operation `delete` sends."""

KEYS = "key_value_store.keys"
"""The operation `keys` sends."""

KEY = "$/key"
"""The notification that carries one of its keys."""


@dataclass(frozen=True)
class StoredBytes:
    """Bytes a Key-value store keeps, with their content type."""

    content: bytes
    """The bytes."""
    content_type: str
    """Their content type, such as `image/png`."""


class AsyncStoreHost(Protocol):
    """What an async Run's store reaches its engine through: the Run's host."""

    @property
    def id(self) -> RunId:
        """The Run each of its calls names."""
        ...

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result, as `AsyncHandleHost.call` does."""
        ...

    def read(
        self,
        operation: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], str | None],
    ) -> AsyncSequence[str]:
        """Sends the Sequence `operation`, read with `async for`."""
        ...


class StoreHost(Protocol):
    """What a sync Run's store reaches its engine through: the Run's host."""

    @property
    def id(self) -> RunId:
        """The Run each of its calls names."""
        ...

    def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result, as `HandleHost.call` does."""
        ...

    def read(
        self,
        operation: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], str | None],
    ) -> Sequence[str]:
        """Sends the Sequence `operation`, read with `for`."""
        ...


@dataclass(frozen=True)
class _Calls:
    """The params of a store's calls, which its two APIs send."""

    run_id: RunId
    """The Run whose store it is."""
    attempt: KeyValueStoreAttempt | None
    """The crawl attempt its handler's calls name; `None` for the program's Run."""

    def key_params(self, key: str) -> KeyValueStoreKeyParams:
        """What `get`, `get_bytes` and `delete` send for `key`."""
        return KeyValueStoreKeyParams.model_construct(None, **self._on(key))

    def set_params(self, key: str, value: JSONValue) -> KeyValueStoreSetParams:
        """What `set` sends for `key`."""
        return KeyValueStoreSetParams.model_construct(
            None, **self._on(key), value=value
        )

    def set_bytes_params(
        self, key: str, content: bytes, content_type: str
    ) -> KeyValueStoreSetBytesParams:
        """What `set_bytes` sends for `key`."""
        return KeyValueStoreSetBytesParams.model_construct(
            None,
            **self._on(key),
            content=KeyValueStoreContent(base64.b64encode(content).decode("ascii")),
            content_type=content_type,
        )

    def keys_params(self, prefix: str | None) -> dict[str, object]:
        """What `keys` sends: its prefix, left out when `None`, and no attempt."""
        params: dict[str, object] = {"run_id": self.run_id.root}
        if prefix is not None:
            params["prefix"] = prefix
        return params

    def _on(self, key: str) -> dict[str, Any]:
        """The fields naming the Run, `key`, and the attempt when there is one."""
        on = {} if self.attempt is None else {"attempt": self.attempt}
        return {"run_id": self.run_id, "key": key, **on}


def _key_of(method: str, params: object) -> str | None:
    """The key a `$/key` notification carries; `None` for another notification."""
    if method != KEY:
        return None
    members = object_members(params)
    key = None if members is None else members.get("key")
    return key if isinstance(key, str) else None


def _on_wire(key: str, value: object, schema: object) -> JSONValue:
    """`value` as `set` sends it for `key`: encoded by `schema`, or its own model.

    Raises:
        ValueInvalidError: It holds something that is not JSON, or its schema
            would not parse its JSON back.
        SchemaValidatorFailedError: A validator raised something other than a
            rejection.
    """
    codec: SchemaCodec[Any] | None = None
    if schema is not None:
        codec = schema_codec(cast("TypeForm[Any]", schema))
    elif isinstance(value, BaseModel):
        codec = schema_codec(type(value))
    if codec is None:
        refused = non_json(value)
        if refused is not None:
            raise _not_json(key, ValueNotJSONError(refused))
        return cast("JSONValue", value)
    try:
        return codec.encode(value)
    except ValueNotJSONError as refused_value:
        raise _not_json(key, refused_value) from None
    except SchemaRejectedError as rejected:
        raise _rejected(
            key, "given", rejected, "Give a value the schema accepts."
        ) from None


def _read_back(key: str, result: object, schema: object) -> object:
    """What `get` answers for `key`: its value, parsed by `schema` when one is given.

    Raises:
        ValueInvalidError: The schema rejects the stored value.
        SchemaValidatorFailedError: A validator raised something other than a
            rejection.
    """
    stored = cast("JSONValue", result_of(KeyValueStoreValue, result).value)
    if schema is None or stored is None:
        return stored
    codec: SchemaCodec[object] = schema_codec(cast("TypeForm[object]", schema))
    try:
        return codec.decode(stored)
    except SchemaRejectedError as rejected:
        raise _rejected(
            key,
            "stored",
            rejected,
            "Read it with the schema it was stored under, or without one, and "
            "store it again in the new shape.",
        ) from None


def _bytes_of(result: object) -> StoredBytes | None:
    """What `get_bytes` answers: the key's bytes, or `None` when it is absent."""
    stored = result_of(KeyValueStoreBytesValue, result).bytes
    if stored is None:
        return None
    return StoredBytes(base64.b64decode(stored.content.root), stored.content_type)


def _existed(result: object) -> bool:
    """What `delete` answers: whether the key existed."""
    return result_of(KeyValueStoreDeleted, result).existed


def _not_json(key: str, refused: ValueNotJSONError) -> ValueInvalidError:
    """`value_invalid` for the value of `key` that is not JSON."""
    return _refused(
        f"The Key-value store refused the value of {_quoted(key)}: {refused}.",
        key,
        reason="value_not_json",
        path=refused.path,
        fix="Encode it as JSON first: a `str`, an `int`, a finite `float`, a `bool`, "
        "`None`, a `list` or a `dict` with `str` keys, or give the schema that "
        "encodes it.",
    )


def _rejected(
    key: str, side: str, rejected: SchemaRejectedError, fix: str
) -> ValueInvalidError:
    """`value_invalid` for the value of `key` its schema rejected, on `side`."""
    return _refused(
        f"The schema rejected the value {side} for {_quoted(key)}: {rejected}.",
        key,
        reason="value_schema",
        path=rejected.issues[0].path if rejected.issues else "",
        fix=fix,
    )


def _refused(
    message: str, key: str, *, reason: str, path: str, fix: str
) -> ValueInvalidError:
    return ValueInvalidError(
        message,
        fix=fix,
        details={"key": key, "refused": "value", "reason": reason, "path": path},
    )


def _quoted(key: str) -> str:
    return json.dumps(key, ensure_ascii=False)


class AsyncKeyValueStore:
    """A Run's Key-value store, in the async API. A program never constructs one.

    A key is 1 to 512 Unicode scalar values with no control character;
    another fails with `value_invalid`, naming it. Through a crawl handler's
    `ctx.run.key_value_store` a write is staged with the attempt; through the
    program's own Run value it commits at once, whatever the attempt becomes.
    """

    def __init__(
        self, host: AsyncStoreHost, attempt: KeyValueStoreAttempt | None = None
    ) -> None:
        """Takes its Run's host and, in a handler, the attempt its calls name."""
        self._host = host
        self._calls = _Calls(host.id, attempt)

    @overload
    async def get(self, key: str) -> JSONValue: ...
    @overload
    async def get(self, key: str, schema: JSONSchema) -> JSONValue: ...
    @overload
    async def get(self, key: str, schema: TypeForm[T]) -> T | None: ...
    async def get(
        self, key: str, schema: JSONSchema | TypeForm[T] | None = None
    ) -> JSONValue | T:
        """The JSON value stored for `key`, or `None` when the key is absent.

        Answers the JSON value a Run's Key-value store holds for a key, as
        committed, or null when the key is absent. A key holding bytes fails with
        `value_invalid`, naming `getBytes`. Takes no writer lock, so it reads a Run no
        program holds without resuming it. Through a crawl attempt (`attempt`), it
        reads what the attempt staged first.

        Experimental: outside semver until it is promoted (ADR 0030).

        Given a schema, it answers the value as that schema parses it, and a
        stored value the schema rejects fails with `value_invalid`, naming the
        first issue's path.
        """
        warn_entry(GET, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        result = await self._host.call(GET, self._calls.key_params(key))
        return cast("JSONValue | T", _read_back(key, result, schema))

    @overload
    async def set(self, key: str, value: StoredValue | BaseModel) -> None: ...
    @overload
    async def set(self, key: str, value: StoredValue, schema: JSONSchema) -> None: ...
    @overload
    async def set(self, key: str, value: T, schema: TypeForm[T]) -> None: ...
    async def set(
        self,
        key: str,
        value: object,
        schema: JSONSchema | TypeForm[Any] | None = None,
    ) -> None:
        """Commits `value`, any JSON but `None` or a Pydantic model, for `key`.

        Commits a JSON value for a key in a live Run's Key-value store, replacing the
        one it held: the last commit wins. A null value fails with `value_invalid`;
        remove a key with `delete`. Through a crawl attempt (`attempt`), the write is
        staged with the attempt and commits or is discarded with it; once the attempt
        ended, it fails with `attempt_ended`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Given a schema, `value` is stored in the form the schema parses back; a
        Pydantic model is stored by its own type without one. A value that is
        not JSON (`float("nan")`, a `Decimal`, a tuple) or that the schema
        rejects fails with `value_invalid`, naming its path, and nothing is sent.
        """
        warn_entry(SET, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        stored = _on_wire(key, value, schema)
        await self._host.call(SET, self._calls.set_params(key, stored))

    async def get_bytes(self, key: str) -> StoredBytes | None:
        """The bytes stored for `key`, or `None` when the key is absent.

        Answers the bytes and content type a Run's Key-value store holds for a key, as
        committed, or null when the key is absent. A key holding JSON fails with
        `value_invalid`, naming `get`. Takes no writer lock. Through a crawl attempt
        (`attempt`), it reads what the attempt staged first.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(GET_BYTES, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return _bytes_of(await self._host.call(GET_BYTES, self._calls.key_params(key)))

    async def set_bytes(self, key: str, content: bytes, content_type: str) -> None:
        """Commits `content`, of `content_type`, for `key`.

        Commits bytes with their content type for a key in a live Run's Key-value
        store, replacing what it held: the last commit wins. Bytes over 1 MiB are kept
        as a file in the Run's folder, which its Checkpoints carry.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(SET_BYTES, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        params = self._calls.set_bytes_params(key, content, content_type)
        await self._host.call(SET_BYTES, params)

    async def delete(self, key: str) -> bool:
        """Removes `key`, answering whether it existed.

        Removes a key from a live Run's Key-value store and answers whether it existed.
        Through a crawl attempt (`attempt`), the removal is staged with the attempt, and
        whether the key existed counts what the attempt staged.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(DELETE, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return _existed(await self._host.call(DELETE, self._calls.key_params(key)))

    def keys(self, *, prefix: str | None = None) -> AsyncSequence[str]:
        """The committed keys that start with `prefix`, every key when left out.

        Sends the committed keys of a Run's Key-value store that start with
        `prefix`, in the order of their Unicode scalar values, in `$/key`
        notifications, one per credit granted with `$/credit`, then answers how many
        it sent. Takes no writer lock.

        Experimental: outside semver until it is promoted (ADR 0030).

        The call is sent at once and read with `async for`; leaving the loop,
        `aclose()`, `close()` or releasing the iterator cancels it.

        Raises:
            ClosedError: The Run is closed.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        warn_entry(KEYS, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return self._host.read(KEYS, self._calls.keys_params(prefix), _key_of)


class KeyValueStore:
    """A Run's Key-value store, in the sync API. A program never constructs one.

    Its keys and its writes are those of `AsyncKeyValueStore`.
    """

    def __init__(
        self, host: StoreHost, attempt: KeyValueStoreAttempt | None = None
    ) -> None:
        """Takes its Run's host and, in a handler, the attempt its calls name."""
        self._host = host
        self._calls = _Calls(host.id, attempt)

    @overload
    def get(self, key: str) -> JSONValue: ...
    @overload
    def get(self, key: str, schema: JSONSchema) -> JSONValue: ...
    @overload
    def get(self, key: str, schema: TypeForm[T]) -> T | None: ...
    def get(
        self, key: str, schema: JSONSchema | TypeForm[T] | None = None
    ) -> JSONValue | T:
        """The JSON value stored for `key`, or `None` when the key is absent.

        Answers the JSON value a Run's Key-value store holds for a key, as
        committed, or null when the key is absent. A key holding bytes fails with
        `value_invalid`, naming `getBytes`. Takes no writer lock, so it reads a Run no
        program holds without resuming it. Through a crawl attempt (`attempt`), it
        reads what the attempt staged first.

        Experimental: outside semver until it is promoted (ADR 0030).

        Given a schema, it answers the value as `AsyncKeyValueStore.get` does.
        """
        warn_entry(GET, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        result = self._host.call(GET, self._calls.key_params(key))
        return cast("JSONValue | T", _read_back(key, result, schema))

    @overload
    def set(self, key: str, value: StoredValue | BaseModel) -> None: ...
    @overload
    def set(self, key: str, value: StoredValue, schema: JSONSchema) -> None: ...
    @overload
    def set(self, key: str, value: T, schema: TypeForm[T]) -> None: ...
    def set(
        self,
        key: str,
        value: object,
        schema: JSONSchema | TypeForm[Any] | None = None,
    ) -> None:
        """Commits `value`, any JSON but `None` or a Pydantic model, for `key`.

        Commits a JSON value for a key in a live Run's Key-value store, replacing the
        one it held: the last commit wins. A null value fails with `value_invalid`;
        remove a key with `delete`. Through a crawl attempt (`attempt`), the write is
        staged with the attempt and commits or is discarded with it; once the attempt
        ended, it fails with `attempt_ended`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Given a schema, `value` is stored as `AsyncKeyValueStore.set` stores it.
        """
        warn_entry(SET, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        stored = _on_wire(key, value, schema)
        self._host.call(SET, self._calls.set_params(key, stored))

    def get_bytes(self, key: str) -> StoredBytes | None:
        """The bytes stored for `key`, or `None` when the key is absent.

        Answers the bytes and content type a Run's Key-value store holds for a key, as
        committed, or null when the key is absent. A key holding JSON fails with
        `value_invalid`, naming `get`. Takes no writer lock. Through a crawl attempt
        (`attempt`), it reads what the attempt staged first.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(GET_BYTES, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return _bytes_of(self._host.call(GET_BYTES, self._calls.key_params(key)))

    def set_bytes(self, key: str, content: bytes, content_type: str) -> None:
        """Commits `content`, of `content_type`, for `key`.

        Commits bytes with their content type for a key in a live Run's Key-value
        store, replacing what it held: the last commit wins. Bytes over 1 MiB are kept
        as a file in the Run's folder, which its Checkpoints carry.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(SET_BYTES, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        params = self._calls.set_bytes_params(key, content, content_type)
        self._host.call(SET_BYTES, params)

    def delete(self, key: str) -> bool:
        """Removes `key`, answering whether it existed.

        Removes a key from a live Run's Key-value store and answers whether it existed.
        Through a crawl attempt (`attempt`), the removal is staged with the attempt, and
        whether the key existed counts what the attempt staged.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(DELETE, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return _existed(self._host.call(DELETE, self._calls.key_params(key)))

    def keys(self, *, prefix: str | None = None) -> Sequence[str]:
        """The committed keys that start with `prefix`, every key when left out.

        Sends the committed keys of a Run's Key-value store that start with
        `prefix`, in the order of their Unicode scalar values, in `$/key`
        notifications, one per credit granted with `$/credit`, then answers how many
        it sent. Takes no writer lock.

        Experimental: outside semver until it is promoted (ADR 0030).

        The call is sent at once and read with `for`; `close()`, leaving a
        `with` block or releasing the iterator cancels it.

        Raises:
            ClosedError: The Run is closed.
        """
        warn_entry(KEYS, EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        return self._host.read(KEYS, self._calls.keys_params(prefix), _key_of)
