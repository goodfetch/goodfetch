"""Settings completeness test against `foxhound.json` (spec #611's #1500; ADR 0083).

Every Foxhound and Collie setting key of the schema is present in Python's
generated `FoxhoundSettings`, `CrawlSettings` and `LabelSettings` with its exact
allowed scopes, and none that the schema omits (such as configuration-only keys).
Collie's `politeness.proxy.*`, `politeness.sharing` and
`site_preferences.respect_use_signals` stay defined in `goodfetch_collie_client`.
"""

from __future__ import annotations

import json
import typing
from typing import get_args, get_origin

import goodfetch_collie_client.generated.settings as collie_settings_mod
import goodfetch_foxhound.generated.settings as foxhound_settings_mod
import pytest
from goodfetch_test_support import WORKSPACE

_CATALOG: dict[str, list[dict[str, object]]] = json.loads(
    (WORKSPACE / "crates/goodfetch-protocol/generated/foxhound.json").read_text(
        encoding="utf-8"
    )
)


def _walk_settings_typeddict(
    cls: type[object], module_name: str, prefix: str = ""
) -> set[str]:
    """Return every dotted wire setting key on a generated Settings TypedDict."""
    keys: set[str] = set()
    hints = typing.get_type_hints(cls)
    for field_name, annotation in hints.items():
        inner = annotation
        if get_origin(inner) is typing.NotRequired:
            inner = get_args(inner)[0]
        dotted = f"{prefix}.{field_name}" if prefix else field_name
        if (
            isinstance(inner, type)
            and getattr(inner, "__module__", None) == module_name
            and inner.__name__.endswith("Settings")
            and hasattr(inner, "__annotations__")
        ):
            keys.update(_walk_settings_typeddict(inner, module_name, dotted))
        else:
            keys.add(dotted)
    return keys


@pytest.mark.parametrize(
    ("scope", "cls"),
    [
        ("connection", foxhound_settings_mod.FoxhoundSettings),
        ("run", foxhound_settings_mod.CrawlSettings),
        ("label", foxhound_settings_mod.LabelSettings),
    ],
    ids=["connection", "run", "label"],
)
def test_every_schema_setting_is_present_with_its_allowed_scope_and_none_omitted(
    scope: str, cls: type[object]
) -> None:
    """Every catalog setting allowed at `scope` is in `cls`, and no others."""
    expected = sorted(
        str(setting["name"])
        for setting in _CATALOG["settings"]
        if isinstance(scopes := setting.get("scopes"), list) and scope in scopes
    )
    actual = sorted(_walk_settings_typeddict(cls, foxhound_settings_mod.__name__))
    assert actual == expected

    config_only = {
        str(setting["name"])
        for setting in _CATALOG["settings"]
        if setting.get("scopes") == []
    }
    assert not (config_only & set(actual))


def test_collies_politeness_and_site_preferences_stay_in_collie_client() -> None:
    """Collie's keys live in `goodfetch_collie_client`; Foxhound adds its own."""
    collie_keys = _walk_settings_typeddict(
        collie_settings_mod.Settings, collie_settings_mod.__name__
    )
    foxhound_keys = _walk_settings_typeddict(
        foxhound_settings_mod.FoxhoundSettings, foxhound_settings_mod.__name__
    )
    for key in (
        "politeness.proxy.concurrency",
        "politeness.proxy.rate",
        "politeness.sharing",
        "site_preferences.respect_use_signals",
    ):
        assert key in collie_keys
        assert key in foxhound_keys
    assert "politeness.domain.concurrency" not in collie_keys
    assert "politeness.domain.concurrency" in foxhound_keys
    assert "politeness.degraded_max" not in foxhound_keys
