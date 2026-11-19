"""The exception a reply raises: its code's class with the fields every error carries.

Or its group's class, keeping the code, when the code is none the SDK
knows. TypeScript's `errors.test.ts` is its twin.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import TYPE_CHECKING, cast

import pytest
from goodfetch_collie_client.generated.errors import (
    ERROR_CLASSES,
    ERROR_GROUP_CLASSES,
    InternalError,
    ProgramError,
    SettingInvalidError,
)
from goodfetch_collie_client.rpc import error_of_reply

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


_CATALOG = Path(__file__).parents[5] / "crates/goodfetch-protocol/generated/collie.json"
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
        (
            "browser_host_unreachable",
            "BrowserHostUnreachableError",
            "unavailable",
            True,
        ),
        ("browser_host_outdated", "BrowserHostOutdatedError", "unavailable", False),
        (
            "executable_blocked_by_smart_app_control",
            "ExecutableBlockedBySmartAppControlError",
            "refused",
            False,
        ),
        ("confinement_unavailable", "ConfinementUnavailableError", "refused", False),
        ("challenge_unsolved", "ChallengeUnsolvedError", "page", False),
        ("plugin_crashed", "PluginCrashedError", "unavailable", True),
        ("person_required", "PersonRequiredError", "refused", False),
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
