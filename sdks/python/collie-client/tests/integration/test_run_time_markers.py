"""ADR 0030's run-time warning for a deprecated or Experimental member (ADR 0099).

`sample.echo`, the catalog's sample entry, is both deprecated and
Experimental. Each program runs in a process of its own, since the warning
comes once per process, and prints what Python's own warning channel shows
under `-W always`, which repeats a warning at every call. TypeScript's
`markers.test.ts` and Rust's `markers/tests.rs` are its twins, and
`settings.test.ts` and Rust's `settings/tests.rs` for a deprecated setting,
`sample.echo_repeats`.
"""

from __future__ import annotations

import subprocess
import sys

import pytest

SAMPLE_ECHO_TWICE = """
from goodfetch_collie_client.generated.operations import CollieOperations
from goodfetch_collie_client.generated.models import SampleEchoParams


class Echo:
    def call(self, operation, params):
        return {"text": params.text}


operations = CollieOperations(Echo())
operations.sample_echo(SampleEchoParams(text="one"))
operations.sample_echo(SampleEchoParams(text="two"))
"""


HAND_WRITTEN_TWICE = """
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)
from goodfetch_collie_client.markers import warn_entry


def echo():
    warn_entry("sample.echo", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)


echo()
echo()
"""


LISTENED_TWICE = """
from goodfetch_collie_client.generated.handles import Page


class Host:
    id = "page-1"

    def listen(self, operation, params, event, payload, listener):
        return None


page = Page(Host())
page.on("console", print)
page.on("console", print)
"""


DEPRECATED_SETTING_TWICE = """
from goodfetch_collie_client.generated.settings import DEPRECATED_SETTINGS
from goodfetch_collie_client.markers import warn_deprecated_settings


def start(settings):
    warn_deprecated_settings(settings, DEPRECATED_SETTINGS)


start({"sample": {"echo_repeats": 3}})
start({"sample": {"echo_repeats": 2}})
start({"timeout": {"navigation": 1}})
"""


RAW_RUN_LAYERS = """
from goodfetch_collie_client.generated.operations import CollieOperations
from goodfetch_collie_client.generated.models import RunResumeParams


class Refused(Exception):
    pass


class Engine:
    def call(self, operation, params):
        raise Refused(operation)


operations = CollieOperations(Engine())
refused = []
for repeats in {repeats}:
    layer = {"sample": {"echo_repeats": repeats}}
    try:
        operations.run_resume(RunResumeParams(run_id="run-1", settings=layer))
    except Refused as refusal:
        refused.append(refusal.args[0])
assert refused == ["run.resume"] * len({repeats}), refused
"""


def shown(program: str) -> list[str]:
    """The warning lines Python shows while `program` runs in its own process."""
    ran = subprocess.run(
        [sys.executable, "-W", "always", "-c", program],
        capture_output=True,
        check=True,
        text=True,
        timeout=60,
    )
    return [line for line in ran.stderr.splitlines() if "Warning: " in line]


@pytest.mark.covers("story:11-184")
def test_a_deprecated_experimental_member_warns_once_of_each_per_process() -> None:
    """Two calls show one `DeprecationWarning` and one `ExperimentalWarning`."""
    lines = shown(SAMPLE_ECHO_TWICE)

    assert lines == [
        "<string>:12: ExperimentalWarning: `sample.echo` is Experimental: "
        "outside semver until it is promoted (ADR 0030).",
        "<string>:12: DeprecationWarning: `sample.echo` is deprecated. "
        "Since 0.1, removed no earlier than 1.0: A sample entry, which the real "
        "catalog replaces.",
    ]


@pytest.mark.covers("story:11-184")
def test_a_hand_written_member_warns_of_its_entry_markers_once_per_process() -> None:
    """A hand-written member reads its entry's markers from the registries.

    It warns once of each, at its caller's line, as a generated one does.
    """
    lines = shown(HAND_WRITTEN_TWICE)

    assert lines == [
        "<string>:13: ExperimentalWarning: `sample.echo` is Experimental: "
        "outside semver until it is promoted (ADR 0030).",
        "<string>:13: DeprecationWarning: `sample.echo` is deprecated. "
        "Since 0.1, removed no earlier than 1.0: A sample entry, which the real "
        "catalog replaces.",
    ]


@pytest.mark.covers("story:11-184")
def test_a_listener_warns_of_its_events_markers_once_per_process() -> None:
    """Listening twice to an Experimental event warns once, at the caller's line.

    `page.on` itself is Experimental too, which its generated member warns of.
    """
    lines = shown(LISTENED_TWICE)

    assert lines == [
        "<string>:13: ExperimentalWarning: `page.on` is Experimental: "
        "outside semver until it is promoted (ADR 0030).",
        "<string>:13: ExperimentalWarning: `page.console` is Experimental: "
        "outside semver until it is promoted (ADR 0030).",
    ]


@pytest.mark.covers("story:11-182")
def test_setting_a_deprecated_key_warns_once_per_process() -> None:
    """Starting two Runs that set it warns once, at the caller's line.

    `start` checks its settings as `AsyncConnection.start` and `Collie.start`
    do; a generated member that takes settings checks them the same way.
    """
    lines = shown(DEPRECATED_SETTING_TWICE)

    assert lines == [
        "<string>:10: DeprecationWarning: The setting `sample.echo_repeats` is "
        "deprecated. Since 0.1, removed no earlier than 1.0: A sample setting, "
        "which the real catalog replaces.",
    ]


@pytest.mark.covers("story:11-182")
@pytest.mark.parametrize(
    "repeats",
    [pytest.param((3, 2), id="set-twice"), pytest.param((None,), id="cleared")],
)
def test_a_deprecated_key_in_a_raw_run_layer_warns_once_per_process(
    repeats: tuple[int | None, ...],
) -> None:
    """Resuming with raw `settings` that set it, `None` included, warns once.

    The warning is at the caller's line. `run.resume` takes the run layer as
    raw JSON, as `run.export`, `run.retry_failed` and Foxhound's `crawl.start`
    do; the warning is the typed path's.
    """
    program = RAW_RUN_LAYERS.replace("{repeats}", repr(repeats))

    lines = [line for line in shown(program) if "The setting" in line]

    assert lines == [
        "<string>:20: DeprecationWarning: The setting `sample.echo_repeats` is "
        "deprecated. Since 0.1, removed no earlier than 1.0: A sample setting, "
        "which the real catalog replaces.",
    ]
