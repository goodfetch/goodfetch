"""What sends a call and reads its result (ADR 0038).

The interfaces every Product's generated operations take, which the client
package's runtime implements, and what builds a result as its model.
Hand-written, since no catalog entry shapes them.
"""

from __future__ import annotations

from collections.abc import Callable
from functools import cache, partial
from types import NoneType, UnionType
from typing import (
    TYPE_CHECKING,
    Annotated,
    Any,
    Literal,
    Protocol,
    TypeGuard,
    TypeVar,
    Union,
    cast,
    get_args,
    get_origin,
)

from pydantic import AwareDatetime, BaseModel, RootModel

from goodfetch_collie_client.point_in_time import point_in_time
from goodfetch_collie_client.unknown_member import UnknownMember

if TYPE_CHECKING:
    from pydantic.fields import FieldInfo

_Model = TypeVar("_Model", bound=BaseModel)


class AsyncCaller(Protocol):
    """What sends a call for the async methods: the client package's runtime."""

    async def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...


class Caller(Protocol):
    """What sends a call for the sync methods: the client package's runtime."""

    def call(self, operation: str, params: BaseModel) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...


def result_of(model: type[_Model], result: object) -> _Model:
    """Builds a result's model from the JSON object the engine sent, nested ones too.

    Each field is built as the type it declares: a model, a list or a map of
    them, a point in time in UTC, or, for a field that is one of several
    models, the one whose tag and required fields the value carries. Nothing
    is validated, so a field, value or variant a newer engine added never
    raises (ADR 0082): an unknown field stays in its model's `model_extra`,
    a member of an open tagged union the SDK does not know is the union's
    `unknown` member, with the tag it was sent in `tag` (ADR 0079), and any
    other value no declared type matches stays as the engine sent it.
    """
    return model.model_construct(**_fields(model, cast("dict[str, object]", result)))


_Build = Callable[[object], object]
"""What builds a value the engine sent as the type a field declares."""


def _fields(model: type[BaseModel], value: dict[str, object]) -> dict[str, Any]:
    """A model's fields, each built as its declared type; unknown ones as sent."""
    builds = _builds(model)
    return {
        key: build(item) if (build := builds.get(key)) else item
        for key, item in value.items()
    }


@cache
def _builds(model: type[BaseModel]) -> dict[str, _Build]:
    """What builds each field of a model that holds a model or a time, by its key.

    Worked out once per model. A field whose type holds neither is left out:
    its value stays as sent.
    """
    builds: dict[str, _Build] = {}
    for key, field in _fields_by_key(model).items():
        build = _build(field.annotation)
        if build is not None:
            builds[key] = build
    return builds


def _build(annotation: object) -> _Build | None:
    """What builds a value of a type unvalidated; none when it holds no model or time.

    Pydantic gives a field typed `PointInTime` the annotation `AwareDatetime`,
    the alias's first argument, and keeps the rest as the field's metadata.
    """
    origin = get_origin(annotation)
    if origin is Annotated:
        return _build(get_args(annotation)[0])
    if annotation is AwareDatetime:
        return point_in_time
    if origin is Union or origin is UnionType:
        return _build_union([m for m in get_args(annotation) if m is not NoneType])
    if origin is list:
        item = _build(get_args(annotation)[0])
        return None if item is None else partial(_build_list, item)
    if origin is dict:
        item = _build(get_args(annotation)[1])
        return None if item is None else partial(_build_dict, item)
    if isinstance(annotation, type):
        if _is_root_model(annotation):
            return partial(_build_root, annotation, _build(_root(annotation)))
        if issubclass(annotation, BaseModel):
            return partial(_build_model, annotation)
    return None


def _build_union(members: list[object]) -> _Build | None:
    """What builds a value of one of several types, null aside."""
    if len(members) == 1:
        # An optional field is built as its one type, as a required one is.
        one = _build(members[0])
        return None if one is None else partial(_build_optional, one)
    builds = [(member, _build(member)) for member in members]
    if all(build is None for _, build in builds):
        return None
    return partial(_build_one_of, builds)


def _build_optional(build: _Build, value: object) -> object:
    return None if value is None else build(value)


def _build_one_of(builds: list[tuple[object, _Build | None]], value: object) -> object:
    for member, build in builds:
        if _matches(member, value):
            return value if build is None else build(value)
    if not isinstance(value, dict):
        return value
    sent = cast("dict[str, object]", value)
    for member, _ in builds:
        if (unknown := _unknown_member(member)) is not None:
            known = [other for other, _ in builds if other is not member]
            return _build_unknown(*unknown, known, sent)
    return sent


def _unknown_member(member: object) -> tuple[type[UnknownMember], str] | None:
    """An open union's `unknown` member and the field that tags the union."""
    if not (isinstance(member, type) and issubclass(member, UnknownMember)):
        return None
    for key, field in _fields_by_key(member).items():
        if get_origin(field.annotation) is Literal and get_args(field.annotation) == (
            "unknown",
        ):
            return member, key
    return None


def _build_unknown(
    member: type[UnknownMember], tag: str, known: list[object], value: dict[str, object]
) -> object:
    """A member the SDK does not know as the `unknown` one, its tag in `tag`.

    A value tagged by a member it knows, which matched none for another
    reason, stays as sent, as any other value no declared type matches.
    """
    sent = value.get(tag)
    if not isinstance(sent, str) or any(_tagged(other, tag, sent) for other in known):
        return value
    return _build_model(member, {**value, tag: "unknown", "tag": sent})


def _tagged(member: object, tag: str, sent: str) -> bool:
    """Whether a member of a union is the one `sent` names in its field `tag`."""
    if not (isinstance(member, type) and issubclass(member, BaseModel)):
        return False
    field = _fields_by_key(member).get(tag)
    return field is not None and _matches(field.annotation, sent)


def _build_list(build: _Build, value: object) -> object:
    if not isinstance(value, list):
        return value
    return [build(item) for item in cast("list[object]", value)]


def _build_dict(build: _Build, value: object) -> object:
    if not isinstance(value, dict):
        return value
    return {key: build(item) for key, item in cast("dict[str, object]", value).items()}


def _build_root(
    model: type[RootModel[Any]], build: _Build | None, value: object
) -> object:
    return model.model_construct(value if build is None else build(value))


def _build_model(model: type[BaseModel], value: object) -> object:
    if not isinstance(value, dict):
        return value
    return model.model_construct(**_fields(model, cast("dict[str, object]", value)))


def _matches(annotation: object, value: object) -> bool:
    """Whether a value has the JSON shape of a type, as one member of a union.

    A model matches an object that carries its required fields and the values
    its literal fields (its tag) name.
    """
    origin = get_origin(annotation)
    if origin is Annotated:
        return _matches(get_args(annotation)[0], value)
    if origin is Union or origin is UnionType:
        return any(_matches(member, value) for member in get_args(annotation))
    if origin is Literal:
        return any(
            type(literal) is type(value) and literal == value
            for literal in get_args(annotation)
        )
    if origin is list:
        return isinstance(value, list)
    if origin is dict:
        return isinstance(value, dict)
    if annotation is Any:
        return True
    if annotation is NoneType:
        return value is None
    if not isinstance(annotation, type):
        return False
    if _is_root_model(annotation):
        return _matches(_root(annotation), value)
    if issubclass(annotation, BaseModel):
        return isinstance(value, dict) and _carries(
            annotation, cast("dict[str, object]", value)
        )
    if issubclass(annotation, bool):
        return isinstance(value, bool)
    if issubclass(annotation, int):
        return isinstance(value, int) and not isinstance(value, bool)
    if issubclass(annotation, float):
        return isinstance(value, int | float) and not isinstance(value, bool)
    # A string, a string enum, or a type JSON carries as a string, such as a
    # point in time.
    return isinstance(value, str)


def _carries(model: type[BaseModel], value: dict[str, object]) -> bool:
    """Whether an object carries a model's required fields and its tag."""
    for key, field in _fields_by_key(model).items():
        if key not in value:
            if field.is_required():
                return False
        elif get_origin(field.annotation) is Literal and not _matches(
            field.annotation, value[key]
        ):
            return False
    return True


@cache
def _fields_by_key(model: type[BaseModel]) -> dict[str, FieldInfo]:
    """A model's fields by the key the engine sends each under: alias, else name."""
    return {field.alias or name: field for name, field in model.model_fields.items()}


def _is_root_model(annotation: type) -> TypeGuard[type[RootModel[Any]]]:
    """Whether a type is a model holding one value, its root, rather than fields."""
    return issubclass(annotation, RootModel)


def _root(model: type[RootModel[Any]]) -> object:
    """The type a root model declares its value with."""
    return model.model_fields["root"].annotation
