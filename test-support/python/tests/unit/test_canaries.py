"""The Secret canary search (ADR 0067) on the fixture set Rust and TypeScript share."""

import json
from pathlib import Path
from typing import NotRequired, TypedDict, cast

import pytest
from goodfetch_test_support import (
    Canary,
    CanaryHit,
    CanaryOutputUnreadable,
    Encoding,
    NamedOutput,
    Placement,
    SecretKind,
    search_canaries,
)

FIXTURE = Path(__file__).resolve().parents[3] / "fixtures" / "canaries" / "search.json"


class _FixtureCanary(TypedDict):
    id: str
    kind: SecretKind
    value: str
    planted_in: NotRequired[list[str]]


class _FixtureOutput(TypedDict):
    name: str
    text: str


class _FixtureHit(TypedDict):
    canary: str
    kind: SecretKind
    encoding: Encoding
    output: str
    offset: int
    placement: Placement


class _Fixture(TypedDict):
    canaries: list[_FixtureCanary]
    outputs: list[_FixtureOutput]
    hits: list[_FixtureHit]


_FIXTURE = cast("_Fixture", json.loads(FIXTURE.read_text("utf-8")))
CANARIES = [
    Canary(
        canary["id"],
        canary["kind"],
        canary["value"],
        tuple(canary.get("planted_in", [])),
    )
    for canary in _FIXTURE["canaries"]
]


@pytest.mark.parametrize(
    "output",
    _FIXTURE["outputs"],
    ids=[output["name"] for output in _FIXTURE["outputs"]],
)
def test_search_finds_each_hit_of_the_shared_fixture(output: _FixtureOutput) -> None:
    """The search finds exactly the hits the shared fixture lists for the output."""
    hits = search_canaries(CANARIES, [NamedOutput(output["name"], output["text"])])

    expected = [
        CanaryHit(
            hit["canary"],
            hit["kind"],
            hit["encoding"],
            hit["output"],
            hit["offset"],
            hit["placement"],
        )
        for hit in _FIXTURE["hits"]
        if hit["output"] == output["name"]
    ]
    assert hits == expected


def test_search_names_the_file_and_offset_of_a_canary_in_a_folder(
    tmp_path: Path,
) -> None:
    """A folder is searched file by file, each hit naming its file's path."""
    (tmp_path / "logs").mkdir()
    (tmp_path / "clean.txt").write_text("nothing here", "utf-8")
    log = tmp_path / "logs" / "sdk.log"
    log.write_text("Error: token=fixture-session-cookie-2\n", "utf-8")

    hits = search_canaries(CANARIES, [tmp_path])

    assert hits == [
        CanaryHit("session", "session_cookie", "raw", str(log), 13, "leaked")
    ]


def test_search_of_a_missing_path_fails(tmp_path: Path) -> None:
    """A path that cannot be read fails the search."""
    with pytest.raises(CanaryOutputUnreadable):
        search_canaries(CANARIES, [tmp_path / "missing"])


def test_a_canarys_repr_never_shows_its_value() -> None:
    """A canary's `repr` names it by ID and kind only."""
    canary = Canary("token", "endpoint_token", "fixture-token-value")

    assert "fixture-token-value" not in repr(canary)
