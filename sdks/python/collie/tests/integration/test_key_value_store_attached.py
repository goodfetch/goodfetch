"""A Run's Key-value store on an Attached engine (ADR 0103; issues 1284 and 1486).

The engine is Collie's executable of the same commit, three ways: async on
asyncio, async on Trio and through the sync `Collie`. `get` on an absent
key answers `None`, a set value comes back, `delete` answers whether the
key existed, and a top-level `None` fails in the engine with
`value_invalid`, naming `delete`; `get_bytes` answers the bytes and content
type `set_bytes` stored, and neither `get` nor `get_bytes` answers the
other's type (S2). A schema types a value both ways (ADR 0082), and what is
not JSON, or what the schema rejects, is refused in the program. TypeScript's
`key-value-store.test.ts` is its twin.
"""

from collections.abc import Mapping
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, cast

import pytest
from collie_calls import failure, open_collie
from goodfetch_collie import (
    AsyncCollie,
    AsyncKeyValueStore,
    AsyncRun,
    StoredBytes,
    ValueInvalidError,
)
from goodfetch_test_support import ClientCaseRecord, Way, three_ways
from pydantic import BaseModel


class Book(BaseModel):
    """A program's model, its exact price stored as a string."""

    title: str
    price: Decimal


@dataclass
class Shelf:
    """A dataclass a program types a value with."""

    name: str
    books: list[str]


@dataclass
class Opened:
    """A Run opened this way, and its Key-value store, async whatever the way."""

    collie: AsyncCollie
    run: AsyncRun
    store: AsyncKeyValueStore

    async def end(self) -> None:
        """Finishes the Run and closes its Connection."""
        await self.run.finish()
        await self.collie.close()


async def _open(way: Way, env: Mapping[str, str], run_id: str) -> Opened:
    collie = await open_collie(way, env)
    run = await collie.start(run_id=run_id)
    # The store is an attribute of the Run, which the sync way's proxy hands
    # as it is: the sync store, which `way.api` wraps.
    store = run.key_value_store
    return Opened(collie, run, way.api(store, store))


@pytest.mark.covers("story:6-28", "story:6-31", "operation:key_value_store.get")
@pytest.mark.client_case(id="key-value-store-get-absent-answers-null", mode="attached")
@three_ways
async def test_answers_none_for_an_absent_key_a_set_value_and_a_delete(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """A value holding a nested null comes back as it was set."""
    opened = await _open(way, env, "stored")
    store = opened.store

    absent = await store.get("cursor")
    await store.set("cursor", {"page": 3, "seen": [1, None]})
    stored = await store.get("cursor")
    deleted = await store.delete("cursor")
    again = await store.delete("cursor")
    await opened.end()

    with client_case.check("get on an absent key answers null"):
        assert absent is None
    with client_case.check("a set value comes back with its nested null"):
        assert stored == {"page": 3, "seen": [1, None]}
    with client_case.check("delete answers whether the key existed"):
        assert [deleted, again] == [True, False]


@pytest.mark.covers("story:6-29", "error:value_invalid")
@pytest.mark.client_case(id="key-value-store-set-null-refused", mode="attached")
@three_ways
async def test_refuses_a_none_value_naming_delete(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """The type refuses a top-level `None`, and the engine an untyped caller's."""
    opened = await _open(way, env, "nulled")

    refused = await failure(opened.store.set("cursor", cast("Any", None)))
    after = await opened.store.get("cursor")
    await opened.end()

    with client_case.check("the engine refused the null with value_invalid"):
        assert isinstance(refused, ValueInvalidError)
        assert (refused.code, refused.group, refused.retryable) == (
            "value_invalid",
            "program",
            False,
        )
        assert refused.details == {
            "key": "cursor",
            "refused": "value",
            "reason": "value_null",
        }
    with client_case.check("its fix names delete"):
        assert "`delete`" in (refused.fix or "")
    with client_case.check("nothing was stored"):
        assert after is None


@pytest.mark.covers("story:6-30")
@pytest.mark.client_case(id="key-value-store-bytes-round-trip", mode="attached")
@three_ways
async def test_answers_the_bytes_and_content_type_set_bytes_stored(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """Bytes that are no text come back as they were set."""
    opened = await _open(way, env, "blobs")
    logo = b"\x89PNG\r\n\x1a\n\x00\xff"

    absent = await opened.store.get_bytes("logo")
    await opened.store.set_bytes("logo", logo, "image/png")
    stored = await opened.store.get_bytes("logo")
    await opened.end()

    with client_case.check("getBytes on an absent key answers null"):
        assert absent is None
    with client_case.check(
        "getBytes answers the bytes and content type setBytes stored"
    ):
        assert stored == StoredBytes(content=logo, content_type="image/png")


@pytest.mark.covers("story:6-30", "error:value_invalid")
@pytest.mark.client_case(
    id="key-value-store-read-of-the-other-type-refused", mode="attached"
)
@three_ways
async def test_refuses_to_read_bytes_with_get_and_json_with_get_bytes(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """Each refusal names the reader of the type the key holds."""
    opened = await _open(way, env, "typed")

    await opened.store.set_bytes("logo", b"\x01\x02\x03", "image/png")
    await opened.store.set("cursor", {"page": 2})
    bytes_with_get = await failure(opened.store.get("logo"))
    json_with_get_bytes = await failure(opened.store.get_bytes("cursor"))
    await opened.end()

    with client_case.check("get on bytes fails with value_invalid naming getBytes"):
        assert isinstance(bytes_with_get, ValueInvalidError)
        assert bytes_with_get.details == {
            "key": "logo",
            "refused": "value",
            "reason": "value_bytes",
        }
        assert "`getBytes`" in (bytes_with_get.fix or "")
    with client_case.check("getBytes on JSON fails with value_invalid naming get"):
        assert isinstance(json_with_get_bytes, ValueInvalidError)
        assert json_with_get_bytes.details == {
            "key": "cursor",
            "refused": "value",
            "reason": "value_json",
        }
        assert "`get`" in (json_with_get_bytes.fix or "")


@pytest.mark.covers("story:11-66", "error:value_invalid")
@three_ways
async def test_types_a_value_by_a_model_a_list_a_dataclass_and_refuses_the_rest(
    way: Way, env: Mapping[str, str]
) -> None:
    """A value comes back in its schema's own type, and a refusal names its path."""
    opened = await _open(way, env, "shelved")
    store = opened.store
    dune = Book(title="Dune", price=Decimal("12.50"))
    shelf = Shelf(name="sf", books=["Dune"])

    await store.set("book", dune)
    await store.set("books", [dune], list[Book])
    await store.set("shelf", shelf, Shelf)
    book = await store.get("book", Book)
    books = await store.get("books", list[Book])
    read_shelf = await store.get("shelf", Shelf)
    as_json = await store.get("book")
    absent = await store.get("absent", Book)
    not_json = await failure(store.set("price", {"amount": float("nan")}))
    rejected = await failure(
        store.set("book", Book.model_construct(title="Dune", price="free"), Book)
    )
    other = await failure(store.get("shelf", Book))
    unchanged = await store.get("book")
    await opened.end()

    assert (book, books, read_shelf, absent) == (dune, [dune], shelf, None)
    assert as_json == {"title": "Dune", "price": "12.50"}
    assert [(type(error), error.details) for error in (not_json, rejected, other)] == [
        (
            ValueInvalidError,
            {"key": key, "refused": "value", "reason": reason, "path": path},
        )
        for key, reason, path in (
            ("price", "value_not_json", "/amount"),
            ("book", "value_schema", "/price"),
            ("shelf", "value_schema", "/title"),
        )
    ]
    assert unchanged == as_json
