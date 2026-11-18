"""Limits in code (ADR 0080): the typed Limits a program gives, as the wire lists them.

A price's `Money` turns into exact millionths of its currency, and every
amount that cannot turn exactly is refused before anything is sent.
TypeScript's `limits.test.ts` is its twin.
"""

from __future__ import annotations

from decimal import Decimal

import pytest
from goodfetch_collie_client.generated.errors import SettingInvalidError
from goodfetch_collie_client.generated.models import RunLimit
from goodfetch_collie_client.limits import WIRE_MAX, Limits, Money, Prices, limit_list


def refusal(limits: Limits) -> SettingInvalidError:
    """The `setting_invalid` that turning `limits` raises."""
    with pytest.raises(SettingInvalidError) as refused:
        limit_list(limits)
    return refused.value


def wire(limits: Limits | None) -> list[dict[str, object]]:
    """`limits` as the wire carries them."""
    return [
        limit.model_dump(mode="json", exclude_unset=True)
        for limit in limit_list(limits)
    ]


@pytest.mark.covers("story:11-51")
def test_lists_each_count_then_each_price_in_millionths_by_usage_kind() -> None:
    """Counts go as given, prices in millionths, each in Usage kind order."""
    limits = Limits(
        page_crashes=0,
        model_calls=100,
        browser_time=60_000,
        price=Prices(
            paid_solves=Money(Decimal("2.5"), "USD"),
            proxy_bytes=Money(Decimal("0.000001"), "EUR"),
        ),
    )

    assert wire(limits) == [
        {"usage_kind": "model_calls", "amount": 100},
        {"usage_kind": "browser_time", "amount": 60_000},
        {"usage_kind": "page_crashes", "amount": 0},
        {"usage_kind": "proxy_bytes", "currency": "EUR", "amount": 1},
        {"usage_kind": "paid_solves", "currency": "USD", "amount": 2_500_000},
    ]


@pytest.mark.covers("story:11-51")
@pytest.mark.parametrize(
    ("amount", "millionths"),
    [
        pytest.param("3", 3_000_000, id="a whole amount"),
        pytest.param("0.123456", 123_456, id="six decimals"),
        pytest.param("1.1234560000", 1_123_456, id="zeros past the sixth decimal"),
        pytest.param("5E+2", 500_000_000, id="an exponent"),
        pytest.param("0E-12", 0, id="zero, however written"),
        pytest.param(
            "18446744073709.551615", WIRE_MAX, id="the largest the wire carries"
        ),
    ],
)
def test_turns_an_amount_exactly(amount: str, millionths: int) -> None:
    """A `Decimal` with at most six significant decimals turns exactly."""
    limits = Limits(price=Prices(model_tokens=Money(Decimal(amount), "USD")))

    assert limit_list(limits) == [
        RunLimit(usage_kind="model_tokens", currency="USD", amount=millionths)
    ]


@pytest.mark.covers("story:11-51")
def test_lists_nothing_for_no_limits() -> None:
    """None, empty Limits and empty prices list nothing."""
    assert [
        limit_list(None),
        limit_list(Limits()),
        limit_list(Limits(price=Prices())),
    ] == [[], [], []]


@pytest.mark.covers("story:11-51")
@pytest.mark.parametrize(
    ("money", "reason"),
    [
        pytest.param(
            Money(Decimal("0.0000001"), "USD"),
            "millionth",
            id="a finer amount than a millionth",
        ),
        pytest.param(
            Money(Decimal("18446744073709.551616"), "USD"),
            "than the wire carries",
            id="an amount past what the wire carries",
        ),
        pytest.param(Money(Decimal("NaN"), "USD"), "no amount", id="not a number"),
        pytest.param(Money(Decimal("Infinity"), "USD"), "no amount", id="infinity"),
        pytest.param(Money(Decimal(-1), "USD"), "no amount", id="a negative amount"),
        pytest.param(Money(Decimal("-0"), "USD"), "no amount", id="a negative zero"),
        pytest.param(
            Money(Decimal(1), "usd"),
            "ISO 4217",
            id="a currency that is no ISO 4217 code",
        ),
        pytest.param(
            Money(0.5, "USD"),  # type: ignore[arg-type]
            "never a float",
            id="a float amount",
        ),
        pytest.param(
            Money("0.5", "USD"),  # type: ignore[arg-type]
            "never a float or a string",
            id="a string amount",
        ),
        pytest.param(
            Decimal("0.5"),
            "a price is a Money",
            id="no Money",
        ),
    ],
)
def test_refuses_a_price_as_setting_invalid(money: Money, reason: str) -> None:
    """The refusal names the price's key and why."""
    refused = refusal(Limits(price=Prices(paid_solves=money)))

    assert (refused.code, refused.group) == ("setting_invalid", "program")
    assert refused.details["key"] == "limits.price.paid_solves"
    assert reason in str(refused.details["reason"])


@pytest.mark.covers("story:11-51")
@pytest.mark.parametrize(
    "count",
    [
        pytest.param(1.5, id="a fraction"),
        pytest.param(-1, id="a negative count"),
        pytest.param(WIRE_MAX + 1, id="a count past what the wire carries"),
        pytest.param(True, id="a bool"),
        pytest.param(Decimal(3), id="a Decimal"),
    ],
)
def test_refuses_a_count_as_setting_invalid(count: object) -> None:
    """A count is a whole `int` from 0 to the wire's largest."""
    refused = refusal(Limits(requests=count))  # type: ignore[arg-type]

    assert refused.details["key"] == "limits.requests"
