"""Results keep what the SDK does not know, and inputs refuse it (ADR 0082)."""

import pydantic
import pytest
from goodfetch_collie_client.generated.models import SampleEchoParams, SampleEchoResult
from goodfetch_collie_client.generated.operations import CollieOperations
from pydantic import BaseModel


class NewerEngine:
    """A caller answering as an engine one minor newer than the SDK would."""

    def call(self, operation: str, params: BaseModel) -> object:
        """Answers every call with the fields this SDK knows and one it does not."""
        del operation, params
        return {"text": "hi", "added_in_a_minor": 1}


def test_result_from_a_newer_engine_keeps_its_unknown_field() -> None:
    """A field a newer engine added neither raises nor is dropped (ADR 0077, G3)."""
    operations = CollieOperations(NewerEngine())

    result = operations.sample_echo(SampleEchoParams(text="hi"))  # type: ignore[deprecated]

    assert result == SampleEchoResult.model_construct(text="hi", added_in_a_minor=1)
    assert result.model_extra == {"added_in_a_minor": 1}


def test_params_refuse_an_unknown_field() -> None:
    """A misspelled input field is an error, never ignored (ADR 0077, G3)."""
    with pytest.raises(pydantic.ValidationError) as refused:
        SampleEchoParams.model_validate({"text": "hi", "txet": 1})

    assert [(error["type"], error["loc"]) for error in refused.value.errors()] == [
        ("extra_forbidden", ("txet",))
    ]
