"""The exception a reply raises when its Error code is none the SDK knows.

Its group's class, keeping the code. TypeScript's `errors.test.ts` is its
twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
from goodfetch_collie_client.generated.errors import InternalError, ProgramError
from goodfetch_collie_client.rpc import error_of_reply

if TYPE_CHECKING:
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
