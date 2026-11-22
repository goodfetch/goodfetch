"""Settings in code (ADR 0083): a scope's `settings` as the wire takes them.

Each scope's settings are typed by the generated `Settings`, `RunSettings`,
`SessionSettings` and `PageSettings`, nested by the wire's own `snake_case`
segments, so the walk only turns values: a `timedelta` goes in whole
milliseconds, rounded up as a call's own `timeout` is (ADR 0080), and every
other value as the program wrote it, `None` included, which clears. The SDK
checks no value the engine owns: what the types refuse and a program still
passes reaches the engine, which refuses it with `setting_unknown` or
`setting_invalid`. TypeScript's `settings.ts` is its twin.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from datetime import timedelta
from typing import cast

from goodfetch_collie_client.handle import whole_milliseconds

__all__ = ["settings_on_wire"]


def settings_on_wire(settings: Mapping[str, object]) -> dict[str, object]:
    """`settings` as the wire takes them: by segment, durations in milliseconds."""
    return {segment: _on_wire(value) for segment, value in settings.items()}


def _on_wire(value: object) -> object:
    """One value as the wire takes it."""
    if isinstance(value, timedelta):
        return whole_milliseconds(value)
    if isinstance(value, Mapping):
        entries = cast("Mapping[object, object]", value)
        return {key: _on_wire(inner) for key, inner in entries.items()}
    if isinstance(value, list | tuple):
        items = cast("Sequence[object]", value)
        return [_on_wire(inner) for inner in items]
    return value
