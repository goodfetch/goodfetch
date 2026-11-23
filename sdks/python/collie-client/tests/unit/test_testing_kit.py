"""The testing kit's isolations (ADR 0105): the test engine's settings and environment.

Rust's unit tests of `goodfetch-collie-client`'s and `goodfetch-collie`'s
`testing` modules are their twins.
"""

from __future__ import annotations

import os
from datetime import timedelta
from typing import TYPE_CHECKING

from goodfetch_collie_client import testing
from goodfetch_collie_client.settings import settings_on_wire

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.settings import Settings

ENGINE_PATH = "GOODFETCH_COLLIE_ENGINE_PATH"
"""Collie's engine-path variable."""

SHORT_TIMEOUTS = {
    "action": 5_000,
    "navigation": 5_000,
    "wait": 5_000,
    "first_byte": 5_000,
    "close": 5_000,
}
"""The kit's five durations on the wire, in milliseconds."""


def test_only_the_hosts_resolve_maps_to_loopback_are_allowed_and_reached() -> None:
    """A host with one address that is not loopback is neither."""
    settings = testing.isolated(
        {
            "network": {
                "resolve": {
                    "app.test": ["127.0.0.1"],
                    "api.test": ["::1"],
                    "mapped.test": ["::ffff:127.0.0.1"],
                    "mixed.test": ["127.0.0.1", "192.0.2.1"],
                    "public.test": ["192.0.2.7"],
                    "empty.test": [],
                }
            }
        }
    )

    loopback = ["api.test", "app.test", "mapped.test"]
    assert settings_on_wire(settings)["egress"] == {
        "offline": True,
        "allowed_domains": loopback,
        "private_ranges": loopback,
    }


def test_with_no_resolve_no_host_is_allowed_and_the_durations_are_short() -> None:
    """Every host is refused, and each of the five durations is 5 s."""
    wire = settings_on_wire(testing.isolated({}))

    assert wire == {
        "egress": {"offline": True, "allowed_domains": [], "private_ranges": []},
        "timeout": SHORT_TIMEOUTS,
    }


def test_a_test_s_own_setting_takes_the_place_of_the_kit_s() -> None:
    """Key by key, the test's settings left unchanged."""
    given: Settings = {
        "egress": {"offline": False, "allowed_domains": ["*"]},
        "timeout": {"navigation": timedelta(minutes=1)},
    }

    wire = settings_on_wire(testing.isolated(given))

    assert wire == {
        "egress": {"offline": False, "allowed_domains": ["*"], "private_ranges": []},
        "timeout": {**SHORT_TIMEOUTS, "navigation": 60_000},
    }
    assert given == {
        "egress": {"offline": False, "allowed_domains": ["*"]},
        "timeout": {"navigation": timedelta(minutes=1)},
    }


def test_the_engine_reads_its_home_and_the_caller_s_browsers_and_engine_alone() -> None:
    """Every other variable of the caller is left out, `GOODFETCH_URL` included."""
    caller = {
        "GOODFETCH_HOME": "/caller/home",
        ENGINE_PATH: "/opt/goodfetch-collie",
        "GOODFETCH_URL": "ws://127.0.0.1:1",
        "PATH": "/usr/bin",
    }

    environment = testing.engine_environment(caller, "/tmp/test-home", ENGINE_PATH)

    assert environment == {
        "GOODFETCH_HOME": "/tmp/test-home",
        "GOODFETCH_BROWSERS_PATH": os.path.join("/caller/home", "browsers"),
        ENGINE_PATH: "/opt/goodfetch-collie",
    }


def test_the_caller_s_browsers_path_takes_the_place_of_its_home_s_cache() -> None:
    """`GOODFETCH_BROWSERS_PATH` names the cache, an engine path left unset."""
    caller = {
        "GOODFETCH_HOME": "/caller/home",
        "GOODFETCH_BROWSERS_PATH": "/prepared/browsers",
    }

    environment = testing.engine_environment(caller, "/tmp/test-home", ENGINE_PATH)

    assert environment == {
        "GOODFETCH_HOME": "/tmp/test-home",
        "GOODFETCH_BROWSERS_PATH": "/prepared/browsers",
    }
