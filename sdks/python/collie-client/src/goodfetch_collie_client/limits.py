"""Limits in code (ADRs 0041 and 0080).

What `connect()` and a Run's start take, typed per Usage kind, with a price
as an exact `Money`. The SDK turns them into the wire's list, each
`{usage_kind, currency?, amount}` with a price in millionths of its
currency, before anything is sent, and an amount it cannot turn exactly
fails the call that carries it with `setting_invalid`. No member raises a
Limit: only a person does. TypeScript's `limits.ts` is its twin.
"""

from __future__ import annotations

import dataclasses
import re
from dataclasses import dataclass
from decimal import Decimal
from typing import TYPE_CHECKING, cast

from goodfetch_collie_client.generated.errors import SettingInvalidError
from goodfetch_collie_client.generated.models import RunLimit

if TYPE_CHECKING:
    from collections.abc import Iterator

DECIMALS = 6
"""Decimals a millionth carries."""

WIRE_MAX = 2**64 - 1
"""The largest amount the wire carries, an unsigned 64-bit integer."""

_CURRENCY = re.compile(r"[A-Z]{3}")


@dataclass(frozen=True)
class Money:
    """A price: an exact `Decimal` amount and its currency.

    Such as `Money(Decimal("0.50"), "USD")`. A float is refused, so no float
    rounding reaches a cap.
    """

    amount: Decimal
    """The amount, exact to a millionth of the currency."""
    currency: str
    """An ISO 4217 code, such as `USD`."""


@dataclass(frozen=True, kw_only=True)
class Prices:
    """What each Usage kind may cost, as `Money`."""

    model_tokens: Money | None = None
    """Model tokens, input and output together."""
    model_calls: Money | None = None
    """Model calls."""
    browser_time: Money | None = None
    """Browser time."""
    proxy_bytes: Money | None = None
    """Bytes through a proxy."""
    paid_solves: Money | None = None
    """Paid challenge solves."""
    requests: Money | None = None
    """Requests."""
    page_crashes: Money | None = None
    """Page crashes."""


@dataclass(frozen=True, kw_only=True)
class Limits:
    """The Limits a program sets on its Connection or on a Run (ADR 0080).

    A count per Usage kind, and a price per Usage kind in `price`, such as
    `Limits(model_calls=100, price=Prices(paid_solves=Money(Decimal("2.50"),
    "USD")))`. Each only lowers what covers that work. Paid fetches are
    Foxhound's: a Collie engine refuses them with `not_in_product`.
    """

    model_tokens: int | None = None
    """Model tokens, input and output together."""
    model_calls: int | None = None
    """Model calls."""
    browser_time: int | None = None
    """Browser time, in milliseconds."""
    proxy_bytes: int | None = None
    """Bytes through a proxy."""
    paid_solves: int | None = None
    """Paid challenge solves."""
    requests: int | None = None
    """Requests."""
    page_crashes: int | None = None
    """Page crashes."""
    price: Prices | None = None
    """What each Usage kind may cost."""


def limit_list(limits: Limits | None) -> list[RunLimit]:
    """The wire's list of `limits`: each count, then each price, by Usage kind.

    Empty for none.

    Raises:
        SettingInvalidError: A count is no whole number from 0 to 2^64 - 1, or
            a price is no `Money` that turns exactly into millionths.
    """
    if limits is None:
        return []
    counts = [
        RunLimit(usage_kind=kind, amount=_count(f"limits.{kind}", value))
        for kind, value in _given(limits)
        if kind != "price"
    ]
    prices = [
        _price(f"limits.price.{kind}", kind, value)
        for kind, value in _given(limits.price)
    ]
    return counts + prices


def _given(fields: Limits | Prices | None) -> Iterator[tuple[str, object]]:
    """Each field of `fields` that is set, in its order: its Usage kind and value."""
    if fields is None:
        return
    for field in dataclasses.fields(fields):
        value: object = getattr(fields, field.name)
        if value is not None:
            yield field.name, value


def _count(key: str, value: object) -> int:
    """`value` as a count of a Usage kind."""
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or not 0 <= value <= WIRE_MAX
    ):
        raise _invalid(
            key,
            "a count is a whole number from 0 to 2^64 - 1",
            "Give a whole number of the Usage kind.",
        )
    return value


def _price(key: str, kind: str, value: object) -> RunLimit:
    """`value`, a `Money`, as a Limit in millionths of its currency."""
    if not isinstance(value, Money):
        raise _invalid(
            key,
            "a price is a Money",
            'Give it as Money(Decimal("0.50"), "USD").',
        )
    # Read as any object: a caller without a type checker can give any.
    amount = cast("object", value.amount)
    currency = cast("object", value.currency)
    if not isinstance(amount, Decimal):
        raise _invalid(
            key,
            "a Money amount is a Decimal, never a float or a string, so that no "
            "float rounding reaches a Limit",
            f'Give the amount as a Decimal, such as Decimal("{amount}").',
        )
    if not isinstance(currency, str) or not _CURRENCY.fullmatch(currency):
        raise _invalid(
            key,
            "a Money currency is an ISO 4217 code",
            "Give three capital letters, such as USD.",
        )
    if not amount.is_finite() or amount.is_signed():
        raise _invalid(
            key,
            f"`{amount}` is no amount of 0 or more",
            'Give a finite amount of 0 or more, such as Decimal("0.50").',
        )
    millionths = _millionths(amount)
    if millionths is None:
        raise _invalid(
            key,
            f"`{amount}` is finer than a millionth of {currency}",
            "Give at most six decimals.",
        )
    if millionths > WIRE_MAX:
        raise _invalid(
            key,
            f"`{amount}` is more millionths of {currency} than the wire carries",
            "Give a smaller amount.",
        )
    return RunLimit(usage_kind=kind, currency=currency, amount=millionths)


def _millionths(amount: Decimal) -> int | None:
    """A finite `amount` of 0 or more in millionths, exactly; `None` when finer.

    Integer arithmetic on its digits, so no context precision rounds it.
    """
    _, digits, exponent = amount.as_tuple()
    if not isinstance(exponent, int):
        return None
    coefficient = int("".join(map(str, digits)) or "0")
    shift = exponent + DECIMALS
    if shift >= 0:
        return coefficient * 10**shift
    whole, past = divmod(coefficient, 10**-shift)
    return None if past else whole


def _invalid(key: str, reason: str, fix: str) -> SettingInvalidError:
    return SettingInvalidError(
        f"{key}: {reason}", fix=fix, details={"key": key, "reason": reason}
    )
