"""The committed JSON Schemas of test support's file types (ADR 0062).

`xtask codegen` writes them from `goodfetch-test-support`'s Rust types into
`test-support/fixtures/schemas/`, so a description or manifest is checked
against the same contract in Rust, TypeScript and Python.
"""

import json
from pathlib import Path
from typing import cast

import jsonschema
from jsonschema import Draft202012Validator
from jsonschema.exceptions import ValidationError

SCHEMAS = Path(__file__).resolve().parents[3] / "fixtures" / "schemas"
"""`test-support/fixtures/schemas/`, beside this package's folder."""


class SchemaError(Exception):
    """A value fails its committed schema."""


def validate(value: object, schema: str) -> None:
    """Checks `value` against the committed schema file named `schema`.

    Args:
        value: The parsed JSON value.
        schema: The schema's file name, such as `test-env-description.schema.json`.

    Raises:
        SchemaError: The value fails the schema.
    """
    document = cast(
        "dict[str, object]", json.loads((SCHEMAS / schema).read_text("utf-8"))
    )
    try:
        jsonschema.validate(value, document, Draft202012Validator)
    except ValidationError as error:
        message = f"{schema} refuses the value at {error.json_path}: {error.message}"
        raise SchemaError(message) from error
