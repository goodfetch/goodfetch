"""Results keep what the SDK does not know, and inputs refuse it (ADR 0082)."""

import pydantic
import pytest
from goodfetch_collie_client.generated.models import (
    FormatsParams,
    FormatsRequest,
    Link,
    PageId,
    PageParams,
    PageResponse,
    RunId,
    RunParams,
    SampleEchoParams,
    SampleEchoResult,
    StopReason2,
)
from goodfetch_collie_client.generated.operations import CollieOperations
from goodfetch_test_support import EngineAnswering
from pydantic import BaseModel

PAGE = PageId(root="page-1")

RESPONSE = {
    "from_service_worker": False,
    "headers": {"content-type": "text/html"},
    "request_id": "request-1",
    "status": 200,
    "status_text": "OK",
    "url": "https://example.com/",
}


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


def test_a_nested_result_reads_by_attribute() -> None:
    """A nested model is built, as its type hint promises."""
    operations = CollieOperations(EngineAnswering({"response": RESPONSE}))

    navigated = operations.page_go_back(PageParams(page_id=PAGE))

    assert isinstance(navigated.response, PageResponse)
    assert navigated.response.status == 200


def test_an_absent_optional_nested_result_is_none() -> None:
    """An optional nested model the engine sends as null stays None."""
    operations = CollieOperations(EngineAnswering({"response": None}))

    navigated = operations.page_go_back(PageParams(page_id=PAGE))

    assert navigated.response is None


def test_a_list_of_nested_results_reads_by_attribute() -> None:
    """Each item of a list of models is built."""
    link = {"href": "/a", "rel": ["next"], "text": "A", "url": "https://example.com/a"}
    operations = CollieOperations(
        EngineAnswering(
            {
                "base_url": "https://example.com/",
                "links": [link],
                "url": "https://example.com/",
            }
        )
    )

    formats = operations.page_formats(
        FormatsParams(formats=FormatsRequest(), page_id=PAGE)
    )

    assert formats.links is not None
    assert [type(item) for item in formats.links] == [Link]
    assert formats.links[0].href == "/a"


def test_an_unknown_field_of_a_nested_result_is_kept() -> None:
    """A field a newer engine added at any depth is kept, never raised on (ADR 0082)."""
    operations = CollieOperations(
        EngineAnswering({"response": {**RESPONSE, "added_in_a_minor": 1}})
    )

    navigated = operations.page_go_back(PageParams(page_id=PAGE))

    assert navigated.response is not None
    assert navigated.response.model_extra == {"added_in_a_minor": 1}


def test_a_union_result_builds_the_variant_its_tag_names() -> None:
    """A field that is one of several models is built as the one its tag names."""
    operations = CollieOperations(EngineAnswering({"stop_reason": {"kind": "pause"}}))

    report = operations.run_report(RunParams(run_id=RunId(root="run-1")))

    assert report.stop_reason is not None
    assert isinstance(report.stop_reason.root, StopReason2)


def test_a_union_variant_a_newer_engine_added_is_kept_as_sent() -> None:
    """A variant the SDK does not know neither raises nor is dropped (ADR 0082)."""
    operations = CollieOperations(
        EngineAnswering({"stop_reason": {"kind": "added_in_a_minor"}})
    )

    report = operations.run_report(RunParams(run_id=RunId(root="run-1")))

    assert report.stop_reason is not None
    sent: object = report.stop_reason.root
    assert sent == {"kind": "added_in_a_minor"}


def test_params_refuse_an_unknown_field() -> None:
    """A misspelled input field is an error, never ignored (ADR 0077, G3)."""
    with pytest.raises(pydantic.ValidationError) as refused:
        SampleEchoParams.model_validate({"text": "hi", "txet": 1})

    assert [(error["type"], error["loc"]) for error in refused.value.errors()] == [
        ("extra_forbidden", ("txet",))
    ]
