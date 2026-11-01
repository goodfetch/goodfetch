"""The shared snapshot redactions and filters (ADR 0062)."""

import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path

import pytest
from goodfetch_test_support import redact
from inline_snapshot import snapshot

CASES = json.loads(
    (
        Path(__file__).resolve().parents[3] / "fixtures" / "snapshot-filter-cases.json"
    ).read_text(encoding="utf-8")
)["cases"]
"""The shared cases every language's `redact` passes."""


@dataclass(frozen=True)
class Run:
    """One run's values, as an output would print them."""

    id: str
    schedule: str
    instant: str
    duration: str
    folder: str
    port: int
    product_version: str
    commit: str

    def output(self) -> str:
        """The output that prints the run's values."""
        return (
            f"run {self.id} of Schedule {self.schedule} started {self.instant}, "
            f"took {self.duration}\n"
            f"wrote {self.folder}/home/report.json\n"
            f"engine goodfetch-collie {self.product_version} ({self.commit})\n"
            f"proving ground at http://127.0.0.1:{self.port}/pages"
        )

    def redacted(self) -> str:
        """The output under the shared rules, with the run's values."""
        return redact(
            self.output(),
            folder=self.folder,
            product_version=self.product_version,
            commit=self.commit,
        )


FIRST = Run(
    id="01JH8Z5Q3W4X5Y6Z7A8B9C0D1E",
    schedule="01JH8Z5Q3W4X5Y6Z7A8B9C0D1F",
    instant="2026-01-05T09:00:00Z",
    duration="1.5s",
    folder="/tmp/snapshot first é 3kq9Zx",
    port=54_321,
    product_version="0.3.1",
    commit="4f1c2e9a7b3d5f6e8a9b0c1d2e3f4a5b6c7d8e9f",
)

SECOND = Run(
    id="01K0AB1C2D3E4F5G6H7J8K9M0N",
    schedule="01K0AB1C2D3E4F5G6H7J8K9M0P",
    instant="2026-03-29T01:59:59.123+01:00",
    duration="250ms",
    folder="/tmp/snapshot second é Q2w8Lp",
    port=8080,
    product_version="0.4.0",
    commit="0a1b2c3d4e5f60718293a4b5c6d7e8f901234567",
)


def test_two_runs_of_one_output_snapshot_alike_under_the_shared_rules() -> None:
    """Two runs' varying values redact to one snapshot, as in insta and Vitest."""
    first = FIRST.redacted()
    second = SECOND.redacted()

    assert first == snapshot("""\
run [ULID] of Schedule [ULID] started [INSTANT], took [DURATION]
wrote [FOLDER]/home/report.json
engine goodfetch-collie [PRODUCT_VERSION] ([COMMIT])
proving ground at http://127.0.0.1:[PORT]/pages\
""")
    assert second == snapshot("""\
run [ULID] of Schedule [ULID] started [INSTANT], took [DURATION]
wrote [FOLDER]/home/report.json
engine goodfetch-collie [PRODUCT_VERSION] ([COMMIT])
proving ground at http://127.0.0.1:[PORT]/pages\
""")


def test_a_value_outside_the_shared_rules_stays_in_the_snapshot() -> None:
    """A value no rule names is kept, so a change to it fails the snapshot."""
    redacted = redact(
        "proxy https://example.com:8443/ answered token 7c1f in state succeeded",
        folder=FIRST.folder,
    )

    assert redacted == snapshot(
        "proxy https://example.com:8443/ answered token 7c1f in state succeeded"
    )


@pytest.mark.parametrize("case", CASES, ids=[case["description"] for case in CASES])
def test_each_shared_case_redacts_as_its_file_says(case: dict[str, str]) -> None:
    """The shared cases redact alike in Rust, TypeScript and Python."""
    assert redact(case["output"]) == case["redacted"]


@pytest.mark.skipif(
    sys.platform == "win32", reason="a Windows folder name holds no quote"
)
def test_the_tests_folder_redacts_in_each_form_an_output_prints_it(
    tmp_path: Path,
) -> None:
    """The folder redacts as its path, its canonical path and each in a JSON string."""
    real = tmp_path / 'real "q" \\b'
    real.mkdir()
    folder = tmp_path / "link"
    folder.symlink_to(real)
    canonical = os.path.realpath(folder)
    output = " ".join(
        [
            f"{folder}/a {canonical}/b",
            json.dumps(
                {"path": f"{folder}/c"}, ensure_ascii=False, separators=(",", ":")
            ),
            json.dumps(
                {"path": f"{canonical}/d"}, ensure_ascii=False, separators=(",", ":")
            ),
        ]
    )

    assert redact(output, folder=folder) == (
        '[FOLDER]/a [FOLDER]/b {"path":"[FOLDER]/c"} {"path":"[FOLDER]/d"}'
    )
