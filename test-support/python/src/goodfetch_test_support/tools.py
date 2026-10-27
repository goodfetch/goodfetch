"""The test tool manifest (ADRs 0062 and 0094).

`just test` builds the commit's executables and writes the absolute paths of
those the workspace has to a JSON file, whose path it puts in
`GOODFETCH_TEST_TOOLS`. A missing manifest or entry names `just test`.
"""

import json
from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Self, cast

from goodfetch_test_support.schemas import validate

TOOLS_VARIABLE = "GOODFETCH_TEST_TOOLS"
"""The variable naming the test tool manifest."""


class Tool(StrEnum):
    """One executable of the manifest, by its key."""

    COLLIE = "collie"
    FOXHOUND = "foxhound"
    PROVING_GROUND = "proving_ground"
    TEST_ENV = "test_env"
    SCRIPTED_ENGINE = "scripted_engine"
    CONFORMANCE = "conformance"
    STUB_GATEWAY = "stub_gateway"


class ToolsError(Exception):
    """A test cannot find an executable; the message names `just test`."""


@dataclass(frozen=True)
class Tools:
    """The commit's executables, each present when its crate exists."""

    paths: Mapping[Tool, Path]

    @classmethod
    def from_json(cls, value: object) -> Self:
        """Reads a manifest's parsed JSON, checked against its committed schema.

        Raises:
            SchemaError: The value fails `test-tool-manifest.schema.json`.
        """
        validate(value, "test-tool-manifest.schema.json")
        entries = cast("dict[str, str]", value)
        return cls({Tool(key): Path(path) for key, path in entries.items()})

    def path(self, tool: Tool) -> Path:
        """The path of one executable.

        Raises:
            ToolsError: The manifest has no entry for `tool`.
        """
        try:
            return self.paths[tool]
        except KeyError:
            message = (
                f"the test tool manifest has no {tool} entry: run the tests with "
                "`just test`, which builds the commit's executables"
            )
            raise ToolsError(message) from None


def read_tools(environ: Mapping[str, str]) -> Tools:
    """Reads the manifest that `environ`'s `GOODFETCH_TEST_TOOLS` names.

    Raises:
        ToolsError: The variable is unset or the manifest cannot be read.
        SchemaError: The manifest fails its committed schema.
    """
    manifest = environ.get(TOOLS_VARIABLE)
    if manifest is None:
        message = (
            f"{TOOLS_VARIABLE} is not set: run the tests with `just test`, which "
            "builds the commit's executables and writes the test tool manifest"
        )
        raise ToolsError(message)
    try:
        text = Path(manifest).read_text("utf-8")
    except OSError as error:
        message = (
            f"the test tool manifest {manifest} cannot be read ({error}): run the "
            "tests with `just test`, which writes it"
        )
        raise ToolsError(message) from error
    return Tools.from_json(json.loads(text))
