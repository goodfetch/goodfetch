"""The committed schemas' formats, checked as Ajv checks them (ADR 0062)."""

import pytest
from goodfetch_test_support.schemas import SchemaError, validate

DESCRIPTION: dict[str, object] = {
    "folder": "/tmp/x é 1",
    "home": "/tmp/x é 1/home",
    "env": {},
    "seed": 1,
    "start_time": "2026-01-05T09:00:00Z",
    "tools": {},
}
"""A description the committed schema accepts."""


def test_a_description_in_its_formats_validates() -> None:
    """The setup's own description passes."""
    validate(DESCRIPTION, "test-env-description.schema.json")


@pytest.mark.parametrize(
    ("field", "value"),
    [
        pytest.param("start_time", "Monday at nine", id="start_time_not_a_date_time"),
        pytest.param("start_time", "2026-01-05", id="start_time_a_date_alone"),
        pytest.param("seed", 2**53, id="seed_past_2_to_the_53"),
    ],
)
def test_a_description_outside_its_formats_fails(field: str, value: object) -> None:
    """A value breaking its field's format fails, as it does in TypeScript."""
    description: dict[str, object] = {**DESCRIPTION, field: value}

    with pytest.raises(SchemaError):
        validate(description, "test-env-description.schema.json")
