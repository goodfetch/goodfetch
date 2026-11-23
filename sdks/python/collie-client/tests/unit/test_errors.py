"""The exception a reply raises: its code's class with the fields every error carries.

Or its group's class, keeping the code, when the code is none the SDK
knows. TypeScript's `errors.test.ts` is its twin.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime
from typing import TYPE_CHECKING, cast

import pytest
from goodfetch_collie_client._rpc import error_of_reply
from goodfetch_collie_client.generated.errors import (
    ERROR_CLASSES,
    ERROR_GROUP_CLASSES,
    ArtifactDownloadFailedError,
    BlockedError,
    CallCancelledError,
    HookFailedError,
    InternalError,
    PageError,
    ProgramError,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.models import (
    ArtifactDownloadFailedDetails2,
    Pinner1,
    Pinner2,
)
from goodfetch_test_support import WORKSPACE

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.errors import ErrorGroup
    from goodfetch_collie_client.goodfetch_error import GoodfetchError


@pytest.mark.parametrize(
    ("wire_code", "code", "group_class", "group"),
    [
        pytest.param(
            -32602, "invalid_params", ProgramError, "program", id="invalid_params"
        ),
        pytest.param(
            -32601, "method_not_found", ProgramError, "program", id="method_not_found"
        ),
        pytest.param(
            -32700, "parse_error", InternalError, "internal", id="parse_error"
        ),
        pytest.param(
            -32600, "invalid_request", InternalError, "internal", id="invalid_request"
        ),
        pytest.param(
            -32603, "internal_error", InternalError, "internal", id="internal_error"
        ),
    ],
)
def test_a_protocol_failure_raises_its_group_class_keeping_its_code(
    wire_code: int, code: str, group_class: type[GoodfetchError], group: str
) -> None:
    """Invalid params and method not found are `program`, the rest `internal`.

    ADR 0079.
    """
    error = error_of_reply(
        {
            "code": wire_code,
            "message": "refused",
            "data": {"code": code, "retryable": False},
        },
        {},
    )

    assert (isinstance(error, group_class), error.code, error.group) == (
        True,
        code,
        group,
    )


def test_a_code_it_does_not_know_raises_its_group_class_keeping_the_code() -> None:
    """A code a newer engine adds raises the class its JSON-RPC code names.

    ADR 0079.
    """
    error = error_of_reply(
        {
            "code": -32003,
            "message": "the page went away",
            "data": {"code": "a_code_from_a_newer_engine", "retryable": True},
        },
        ERROR_CLASSES,
    )

    assert (isinstance(error, PageError), error.code, error.group, error.retryable) == (
        True,
        "a_code_from_a_newer_engine",
        "page",
        True,
    )


_CATALOG = WORKSPACE / "crates/goodfetch-protocol/generated/collie.json"
_CODES: list[dict[str, object]] = json.loads(_CATALOG.read_text(encoding="utf-8"))[
    "error_codes"
]

_REFUSAL: dict[str, object] = {
    "code": -32001,
    "message": "page.goto's timeout of 0 ms is below the least, 1 ms",
    "data": {
        "code": "setting_invalid",
        "retryable": False,
        "fix": "Pass a timeout of 1 ms or more.",
        "operation": "page.goto",
        "call_id": "call-7",
        "run_id": "run-3",
        "target": "page.get_by_role('button', name='Buy')",
        "url": "https://shop.example/cart",
        "observed": "a button that is disabled",
        "limit": {"key": "timeout", "value": 0, "layer": "run", "origin": "argument"},
        "call_log": ["waiting for get_by_role('button')", "  locator resolved"],
        "usage": {
            "calls": 1,
            "input_tokens": 10,
            "output_tokens": 2,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0,
        },
        "agent_task": {"steps": []},
        "reason": "below_least",
    },
}


@pytest.mark.covers("story:11-30")
def test_a_reply_raises_its_code_class_with_the_fields_every_error_carries() -> None:
    """The common fields leave the details, which keep the code's own."""
    error = error_of_reply(_REFUSAL, ERROR_CLASSES)

    assert isinstance(error, SettingInvalidError)
    assert error.limit is not None
    assert error.usage is not None
    assert {
        "name": error.name,
        "message": str(error),
        "code": error.code,
        "group": error.group,
        "retryable": error.retryable,
        "fix": error.fix,
        "operation": error.operation,
        "call_id": error.call_id,
        "run_id": error.run_id,
        "target": error.target,
        "url": error.url,
        "observed": error.observed,
        "limit": (
            error.limit.key,
            error.limit.value,
            error.limit.layer,
            error.limit.origin,
        ),
        "call_log": error.call_log,
        "usage": error.usage.model_dump(mode="json", exclude_none=True),
        "agent_task": error.agent_task,
        "details": error.details,
    } == {
        "name": "SettingInvalidError",
        "message": _REFUSAL["message"],
        "code": "setting_invalid",
        "group": "program",
        "retryable": False,
        "fix": "Pass a timeout of 1 ms or more.",
        "operation": "page.goto",
        "call_id": "call-7",
        "run_id": "run-3",
        "target": "page.get_by_role('button', name='Buy')",
        "url": "https://shop.example/cart",
        "observed": "a button that is disabled",
        "limit": ("timeout", 0, "run", "argument"),
        "call_log": ["waiting for get_by_role('button')", "  locator resolved"],
        "usage": {
            "calls": 1,
            "input_tokens": 10,
            "output_tokens": 2,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0,
        },
        "agent_task": {"steps": []},
        "details": {"reason": "below_least"},
    }


@pytest.mark.parametrize(
    ("key", "value"),
    [
        ("limit", 10_485_760),
        ("limit", {"kind": "option", "name": "max_redirects", "count": 3}),
        ("url", 404),
        ("call_log", "one line"),
        ("usage", 7),
    ],
)
def test_a_field_of_another_shape_stays_in_the_details(key: str, value: object) -> None:
    """A code's own details may use a common field's name for something else.

    Such as `response_limit_exceeded`'s byte count or a redirect cap's
    `{kind, name, count}`: a value without the field's shape stays there.
    """
    error = error_of_reply(
        {"code": -32003, "message": "too big", "data": {"code": "blocked", key: value}},
        ERROR_CLASSES,
    )

    assert (error.limit, error.url, error.usage, error.details) == (
        None,
        None,
        None,
        {key: value},
    )


@pytest.mark.parametrize(
    ("sent", "retry_after"),
    [
        pytest.param(
            "2026-11-21T10:00:00+02:00",
            datetime(2026, 11, 21, 8, tzinfo=UTC),
            id="a time in another offset",
        ),
        pytest.param(None, None, id="null"),
    ],
)
def test_blocked_reads_its_retry_after_as_a_point_in_time(
    sent: str | None, retry_after: datetime | None
) -> None:
    """The time the site's `Retry-After` asks for, in UTC (ADRs 0092 and 0118).

    `None` when it asks for none; the details keep it as sent.
    """
    error = error_of_reply(
        {
            "code": -32003,
            "message": "blocked",
            "data": {"code": "blocked", "cause": "rate_limit", "retry_after": sent},
        },
        ERROR_CLASSES,
    )

    assert isinstance(error, BlockedError)
    assert (error.retry_after, error.details["retry_after"]) == (retry_after, sent)
    assert error.retry_after is None or error.retry_after.tzinfo == UTC


def test_blocked_with_no_retry_after_has_none() -> None:
    """An engine that sent no `retry_after` leaves the field `None`."""
    error = error_of_reply(
        {"code": -32003, "message": "blocked", "data": {"code": "blocked"}},
        ERROR_CLASSES,
    )

    assert isinstance(error, BlockedError)
    assert error.retry_after is None


@pytest.mark.covers("story:11-48", "error:call_cancelled")
@pytest.mark.parametrize(
    ("sent", "read"),
    [
        pytest.param(
            {"by": "person", "closed": "run", "person": "ada"},
            ("person", "run", "ada"),
            id="a person's Client stopping the Run",
        ),
        pytest.param({"by": "client"}, ("client", None, None), id="nothing closed"),
        pytest.param(
            {
                "by": "a_stopper_from_a_newer_engine",
                "closed": "a_value_from_a_newer_engine",
            },
            ("a_stopper_from_a_newer_engine", "a_value_from_a_newer_engine", None),
            id="values a newer engine names",
        ),
    ],
)
def test_call_cancelled_reads_its_details_typed(
    sent: dict[str, object], read: tuple[str, str | None, str | None]
) -> None:
    """Who stopped the call, what closed under it and for whom (ADRs 0079 and 0113).

    A value a newer engine names reaches the program unchanged.
    """
    error = error_of_reply(
        {
            "code": -32006,
            "message": "the call was stopped under it",
            "data": {"code": "call_cancelled", "retryable": False, **sent},
        },
        ERROR_CLASSES,
    )

    assert isinstance(error, CallCancelledError)
    details = error.typed_details
    assert details is not None
    assert (details.by, details.closed, details.person) == read


@pytest.mark.covers("error:hook_failed")
@pytest.mark.parametrize("stage", ["after_response", "a_stage_from_a_newer_engine"])
def test_hook_failed_reads_its_details_typed(stage: str) -> None:
    """A Hook that raised reads its stage, Hook and exception typed (ADRs 0079, 0104).

    The client package carries them, since a Collie Run's Script Hooks raise
    it too; a stage a newer engine names reaches the program unchanged.
    """
    error = error_of_reply(
        {
            "code": -32001,
            "message": "the Hook checkPrice raised TypeError: price is undefined",
            "data": {
                "code": "hook_failed",
                "retryable": True,
                "stage": stage,
                "hook": "checkPrice",
                "label": None,
                "exception_type": "TypeError",
                "exception_message": "price is undefined",
                "exception_stack": "at checkPrice (crawl.ts:12:5)",
                "wrapped_code": "header_forbidden",
            },
        },
        ERROR_CLASSES,
    )

    assert isinstance(error, HookFailedError)
    details = error.typed_details
    assert details is not None
    assert (
        details.stage,
        details.hook,
        details.label,
        details.exception_type,
        details.exception_message,
        details.exception_stack,
        details.wrapped_code,
    ) == (
        stage,
        "checkPrice",
        None,
        "TypeError",
        "price is undefined",
        "at checkPrice (crawl.ts:12:5)",
        "header_forbidden",
    )


def test_artifact_download_failed_reads_its_pinners_typed_beside_its_failure() -> None:
    """A failed download reads its pinners as a list beside its failure (ADR 0079).

    Its details are one member of the union their failure flattens; its
    `fix` is the error's, which every error carries.
    """
    error = error_of_reply(
        {
            "code": -32006,
            "message": "the download of chromium failed",
            "data": {
                "code": "artifact_download_failed",
                "retryable": True,
                "artifact": {
                    "kind": "browser",
                    "name": "chromium",
                    "build": "141.0.7390.54-linux64",
                },
                "pinned_by": [
                    {"kind": "product", "name": "collie", "version": "0.1.0"},
                    {"kind": "plugin", "name": "stealth", "version": "2.0.0"},
                ],
                "host": "storage.googleapis.com",
                "fix": "Retry, or install beforehand with `collie browser install`.",
                "failure": "http_status",
                "status": 503,
            },
        },
        ERROR_CLASSES,
    )

    assert isinstance(error, ArtifactDownloadFailedError)
    assert error.typed_details is not None
    details = error.typed_details.root
    assert isinstance(details, ArtifactDownloadFailedDetails2)
    assert (details.failure, details.status, details.host, details.artifact.name) == (
        "http_status",
        503,
        "storage.googleapis.com",
        "chromium",
    )
    product, plugin = (pinner.root for pinner in details.pinned_by)
    assert isinstance(product, Pinner1)
    assert isinstance(plugin, Pinner2)
    assert (product.name, product.version.root) == ("collie", "0.1.0")
    assert (plugin.name, plugin.version) == ("stealth", "2.0.0")
    assert error.fix == "Retry, or install beforehand with `collie browser install`."


def test_call_cancelled_naming_no_one_who_stopped_it_has_no_typed_details() -> None:
    """An error a program built without details reads none, rather than failing."""
    assert CallCancelledError("stopped by its program").typed_details is None


def test_an_error_with_no_call_log_has_an_empty_one() -> None:
    """An engine that sent no call log leaves the field empty, never absent."""
    error = error_of_reply(
        {"code": -32003, "message": "blocked", "data": {"code": "blocked"}},
        ERROR_CLASSES,
    )

    assert error.call_log == []


@pytest.mark.covers("story:11-190")
@pytest.mark.parametrize(
    ("code", "class_name", "group", "retryable"),
    [
        pytest.param(
            "browser_host_unreachable",
            "BrowserHostUnreachableError",
            "unavailable",
            True,
            id="browser_host_unreachable",
        ),
        pytest.param(
            "browser_host_outdated",
            "BrowserHostOutdatedError",
            "unavailable",
            False,
            id="browser_host_outdated",
        ),
        pytest.param(
            "executable_blocked_by_smart_app_control",
            "ExecutableBlockedBySmartAppControlError",
            "refused",
            False,
            id="executable_blocked_by_smart_app_control",
        ),
        pytest.param(
            "confinement_unavailable",
            "ConfinementUnavailableError",
            "refused",
            False,
            id="confinement_unavailable",
        ),
        pytest.param(
            "challenge_unsolved",
            "ChallengeUnsolvedError",
            "page",
            False,
            id="challenge_unsolved",
        ),
        pytest.param(
            "plugin_crashed",
            "PluginCrashedError",
            "unavailable",
            True,
            id="plugin_crashed",
        ),
        pytest.param(
            "person_required",
            "PersonRequiredError",
            "refused",
            False,
            id="person_required",
        ),
    ],
)
def test_the_codes_areas_5_7_and_8_add_have_their_class_under_their_group(
    code: str, class_name: str, group: ErrorGroup, retryable: bool
) -> None:
    """Each code has its own class under the group its ADRs fix.

    ADRs 0033, 0035, 0039, 0079 and 0092; Windows' refusal to start a file
    is `executable_blocked_by_smart_app_control` (ADR 0039's note of
    2026-10-20).
    """
    known = ERROR_CLASSES.get(code)
    error = None if known is None else known("failed")

    assert {
        "name": None if error is None else error.name,
        "is_group_class": isinstance(error, ERROR_GROUP_CLASSES[group]),
        "group": None if error is None else error.group,
        "retryable": None if error is None else error.retryable,
    } == {
        "name": class_name,
        "is_group_class": True,
        "group": group,
        "retryable": retryable,
    }


@pytest.mark.covers("story:11-28", "story:11-37")
@pytest.mark.parametrize("code", _CODES, ids=lambda code: str(code["name"]))
def test_each_error_code_has_its_class_under_its_group(code: dict[str, object]) -> None:
    """Every code of the catalog has its class; one named like its group, the group's.

    ADR 0099.
    """
    name, group, retryable = code["name"], code["group"], code["retryable"]
    assert isinstance(name, str)
    group_class = ERROR_GROUP_CLASSES[cast("ErrorGroup", group)]
    known = ERROR_CLASSES.get(name)
    error = (
        group_class(
            "failed",
            code=name,
            group=cast("ErrorGroup", group),
            retryable=cast("bool", retryable),
        )
        if known is None
        else known("failed")
    )

    assert {
        "is_group_class": isinstance(error, group_class),
        "has_own_class": known is not None or name == group,
        "name": error.name.endswith("Error"),
        "code": error.code,
        "group": error.group,
        "retryable": error.retryable,
    } == {
        "is_group_class": True,
        "has_own_class": True,
        "name": True,
        "code": name,
        "group": group,
        "retryable": retryable,
    }
