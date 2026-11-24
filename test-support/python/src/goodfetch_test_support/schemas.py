"""The committed JSON Schemas of test support's file types (ADR 0062).

`xtask codegen` writes them from `goodfetch-test-support`'s Rust types into
`test-support/fixtures/schemas/`, so a description or manifest is checked
against the same contract in Rust, TypeScript and Python.
"""

import json
import re
from datetime import date, datetime
from pathlib import Path
from typing import cast

import jsonschema
from jsonschema import Draft202012Validator, FormatChecker
from jsonschema.exceptions import ValidationError

SCHEMAS = Path(__file__).resolve().parents[3] / "fixtures" / "schemas"
"""`test-support/fixtures/schemas/`, beside this package's folder."""


class SchemaError(Exception):
    """A value fails its committed schema."""


_DATE_TIME = re.compile(
    r"\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:\d{2})"
)
"""RFC 3339's date-time, whose fields `datetime.fromisoformat` then checks."""

FORMATS = FormatChecker()
"""The formats the committed schemas use, checked as TypeScript's Ajv
checks them: `date` and `date-time` as RFC 3339 writes them, schemars's
`uint64` as a non-negative integer below 2^53, `int64` as an integer of
magnitude below 2^53, and `uint8`."""


@FORMATS.checks("date")
def _date(value: object) -> bool:
    if not isinstance(value, str):
        return True
    try:
        date.fromisoformat(value)
    except ValueError:
        return False
    return len(value) == len("2026-01-05")


@FORMATS.checks("date-time")
def _date_time(value: object) -> bool:
    if not isinstance(value, str):
        return True
    if _DATE_TIME.fullmatch(value) is None:
        return False
    try:
        datetime.fromisoformat(value.replace("z", "Z"))
    except ValueError:
        return False
    return True


@FORMATS.checks("uint64")
def _uint64(value: object) -> bool:
    return not isinstance(value, int) or 0 <= value < 2**53


@FORMATS.checks("int64")
def _int64(value: object) -> bool:
    return not isinstance(value, int) or -(2**53) < value < 2**53


@FORMATS.checks("uint8")
def _uint8(value: object) -> bool:
    return not isinstance(value, int) or 0 <= value <= 255


def validate(value: object, schema: str | Path) -> None:
    """Checks `value` against the committed schema file named `schema`.

    Args:
        value: The parsed JSON value.
        schema: The schema's file name in `test-support/fixtures/schemas/`, such
            as `test-env-description.schema.json`, or a schema file's path.

    Raises:
        SchemaError: The value fails the schema.
    """
    document = cast(
        "dict[str, object]", json.loads((SCHEMAS / schema).read_text("utf-8"))
    )
    try:
        jsonschema.validate(
            value, document, Draft202012Validator, format_checker=FORMATS
        )
    except ValidationError as error:
        message = f"{schema} refuses the value at {error.json_path}: {error.message}"
        raise SchemaError(message) from error


def shared_fixture(file: str, schema: str) -> object:
    """Reads the shared fixture `file` of `test-support/fixtures/`.

    Args:
        file: The fixture's file name, such as `locales.json`.
        schema: Its schema's file name, such as `locales.schema.json`.

    Returns:
        The parsed JSON value, checked against the committed schema.

    Raises:
        SchemaError: The fixture fails its schema.
    """
    value: object = json.loads((SCHEMAS.parent / file).read_text("utf-8"))
    validate(value, schema)
    return value
