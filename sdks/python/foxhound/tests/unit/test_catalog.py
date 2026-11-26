"""Foxhound's catalog against this package (ADRs 0095 and 0099).

Each Error code only Foxhound carries has its generated class, under its
group but for `attempt_ended`'s, and each event only Foxhound sends its
payload type, so the catalog gains none without its counterpart here.
TypeScript's `catalog.test.ts` of `@goodfetch/foxhound` is its twin.
"""

from __future__ import annotations

import json
from typing import TYPE_CHECKING, cast

import pytest
from goodfetch_collie_client.generated.errors import ERROR_GROUP_CLASSES
from goodfetch_foxhound.generated.errors import ERROR_CLASSES
from goodfetch_foxhound.generated.events import EVENTS
from goodfetch_test_support import WORKSPACE

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.errors import ErrorGroup

_CATALOG: dict[str, list[dict[str, object]]] = json.loads(
    (WORKSPACE / "crates/goodfetch-protocol/generated/foxhound.json").read_text(
        encoding="utf-8"
    )
)
_CODES = [code for code in _CATALOG["error_codes"] if code["product"] == "foxhound"]


@pytest.mark.parametrize("code", _CODES, ids=lambda code: str(code["name"]))
def test_each_code_only_foxhound_carries_has_its_class_under_its_group(
    code: dict[str, object],
) -> None:
    """Its class raises it with its code, group and retryable flag.

    `attempt_ended`'s alone derives from `BaseException`, outside its group's
    class, so a handler's `except Exception:` lets it through (ADR 0093).
    """
    name, group, retryable = code["name"], code["group"], code["retryable"]
    assert isinstance(name, str)
    known = ERROR_CLASSES.get(name)
    assert known is not None, f"{name} has no generated class"
    error = known("failed")

    assert (
        isinstance(error, ERROR_GROUP_CLASSES[cast("ErrorGroup", group)]),
        error.code,
        error.group,
        error.retryable,
    ) == (name != "attempt_ended", name, group, retryable)


def test_each_event_only_foxhound_sends_has_its_payload_type() -> None:
    """The events registry lists exactly the catalog's Foxhound events."""
    foxhounds = sorted(
        str(event["name"])
        for event in _CATALOG["events"]
        if event["product"] == "foxhound"
    )

    assert foxhounds
    assert sorted(EVENTS) == foxhounds
