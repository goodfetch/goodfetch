"""ADR 0030's run-time warning for a deprecated or Experimental member (ADR 0099).

`sample.echo`, the catalog's sample entry, is both deprecated and
Experimental. Each program runs in a process of its own, since the warning
comes once per process, and prints what Python's own warning channel shows
under `-W always`, which repeats a warning at every call. TypeScript's
`markers.test.ts` and Rust's `markers/tests.rs` are its twins. Story 11-182
is declared with its release-notes list (issue 1256).
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
        "Since 0.1: A sample entry, which the real catalog replaces.",
    ]
