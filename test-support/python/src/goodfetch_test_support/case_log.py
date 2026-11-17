"""The case log of Client conformance cases (ADR 0069).

The plugin writes one JSON Lines entry per time a pytest test ran a case,
with no outcome field, so only `goodfetch-conformance`'s writer derives
outcomes from it. A test carries its case and mode as the marker
`client_case(id="<case>", mode="attached")`, and records its checks and
evidence through the `client_case` fixture, a `ClientCaseRecord`. Each entry
is checked against the bundle's `CaseLogEntry` schema before it is written.
"""

import contextlib
import functools
import json
import platform
import sys
import sysconfig
import tomllib
from collections.abc import Generator, Mapping, Sequence
from pathlib import Path
from typing import cast

import jsonschema
from jsonschema import Draft202012Validator
from jsonschema.exceptions import ValidationError

from goodfetch_test_support.generated.result_document import (
    CaseCheck,
    CaseLogEntry,
    OracleAnswer,
    Platform,
)
from goodfetch_test_support.schemas import FORMATS

CLIENT_CASE_MODES = ("attached", "daemon")
"""The modes a Python test runs a case in (ADR 0069)."""

TEST_PASSED = "the test passed"
"""The check every entry ends with: whether pytest passed the test, so a test
that fails outside its checks fails its case."""

_BUNDLE = (
    Path(__file__).resolve().parents[4]
    / "crates"
    / "goodfetch-protocol"
    / "generated"
    / "bundle.json"
)
"""The bundle of `goodfetch-protocol`'s schemas, which `xtask codegen` writes."""


class CaseListInvalid(Exception):
    """A case list cannot be read, is no TOML, or misses a suite, a name or an ID."""


class CaseLogEntryInvalid(Exception):
    """A case log entry fails the bundle's `CaseLogEntry` schema."""


class ClientCaseUntagged(Exception):
    """A test records a Client conformance case but carries no `client_case` marker."""


def read_client_cases(path: Path) -> dict[str, str]:
    """Each case of the case list at `path`, with its capability.

    The list is in `conformance/clients/cases.toml`'s shape.

    Its other fields are `xtask clients check`'s and the writer's.

    Raises:
        CaseListInvalid: The list cannot be read, is no TOML, has no
            `[suite]` with a name and version, or a capability has no name or
            a case no ID.
    """
    try:
        document = tomllib.loads(path.read_text("utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        message = f"the case list {path} cannot be read as TOML: {error}"
        raise CaseListInvalid(message) from error
    suite = document.get("suite")
    if not (
        isinstance(suite, dict)
        and isinstance(cast("dict[str, object]", suite).get("name"), str)
        and isinstance(cast("dict[str, object]", suite).get("version"), str)
    ):
        message = f"the case list {path} has no [suite] name and version"
        raise CaseListInvalid(message)
    cases: dict[str, str] = {}
    for index, capability in enumerate(_tables(document.get("capability"), path), 1):
        name = capability.get("name")
        if not isinstance(name, str):
            message = f"capability {index} of the case list {path} has no name"
            raise CaseListInvalid(message)
        for at, declared in enumerate(_tables(capability.get("case"), path), 1):
            id_ = declared.get("id")
            if not isinstance(id_, str):
                message = f"case {at} of capability {name} in {path} has no ID"
                raise CaseListInvalid(message)
            cases[id_] = name
    return cases


def _tables(value: object, path: Path) -> list[dict[str, object]]:
    if value is None:
        return []
    if not isinstance(value, list) or not all(
        isinstance(table, dict) for table in cast("list[object]", value)
    ):
        message = f"the case list {path} holds an array that is no array of tables"
        raise CaseListInvalid(message)
    return cast("list[dict[str, object]]", value)


def checked_case_log_entry(value: object) -> CaseLogEntry:
    """`value` as a case log entry.

    Raises:
        CaseLogEntryInvalid: It fails the bundle's schema, such as by setting
            an outcome.
    """
    try:
        jsonschema.validate(
            value,
            _case_log_entry_schema(),
            Draft202012Validator,
            format_checker=FORMATS,
        )
    except ValidationError as error:
        message = (
            f"the case log entry fails the bundle's schema at {error.json_path}: "
            f"{error.message}"
        )
        raise CaseLogEntryInvalid(message) from error
    return CaseLogEntry.model_validate(value)


@functools.cache
def _case_log_entry_schema() -> dict[str, object]:
    """The bundle's `CaseLogEntry` schema, read once."""
    bundle = cast("dict[str, object]", json.loads(_BUNDLE.read_text("utf-8")))
    return {"$ref": "#/$defs/CaseLogEntry", "$defs": bundle["$defs"]}


def host_platform() -> Platform:
    """This host's operating system and architecture, as Rust names them.

    Every result document names them as `std::env::consts` does.
    """
    return rust_platform(platform.system(), platform.machine())


_OS = {"darwin": "macos"}
_ARCH = {"amd64": "x86_64", "arm64": "aarch64", "i386": "x86", "i686": "x86"}


def rust_platform(system: str, machine: str) -> Platform:
    """`platform.system()` and `platform.machine()` as `std::env::consts` names them.

    `Darwin` is `macos`, `AMD64` is `x86_64` and `arm64` is `aarch64`.
    """
    os = system.lower()
    arch = machine.lower()
    return Platform(os=_OS.get(os, os), arch=_ARCH.get(arch, arch))


def host_runtime() -> str:
    """The runtime the cases run on.

    Such as `cpython-3.12`, or `cpython-3.14t` on a free-threaded build.
    """
    return python_runtime(
        sys.implementation.name,
        (sys.version_info.major, sys.version_info.minor),
        free_threaded=bool(sysconfig.get_config_var("Py_GIL_DISABLED")),
    )


def python_runtime(
    implementation: str, version: tuple[int, int], *, free_threaded: bool
) -> str:
    """An interpreter's runtime name: its implementation and version.

    A `t` after the version marks a free-threaded build.
    """
    major, minor = version
    return f"{implementation}-{major}.{minor}{'t' if free_threaded else ''}"


class ClientCaseRecord:
    """Records a case's checks and evidence for its case log entry.

    Each check runs a block of assertions and records whether it held; a
    check that fails still fails the test.
    """

    def __init__(self) -> None:
        """A record with no check and no evidence yet."""
        self.checks: list[CaseCheck] = []
        """Each check the test ran, in order."""
        self._messages: list[object] = []
        self._oracle_answers: list[object] = []
        self._observations: list[object] = []

    @contextlib.contextmanager
    def check(self, name: str) -> Generator[None]:
        """Runs the `with` block as the check `name`, and records whether it held.

        A failure in the block is re-raised.
        """
        try:
            yield
        # `pytest.fail` raises an exception that is no `Exception`.
        except BaseException:
            self.checks.append(CaseCheck(name=name, passed=False))
            raise
        self.checks.append(CaseCheck(name=name, passed=True))

    def message(self, value: object) -> None:
        """Records a scripted message the case exchanged."""
        self._messages.append(value)

    def oracle_answer(self, answer: OracleAnswer) -> None:
        """Records an oracle's answer the case got, with each field it came with."""
        self._oracle_answers.append(answer.model_dump(mode="json", exclude_unset=True))

    def observation(self, value: object) -> None:
        """Records what the Proving ground observed on the server side."""
        self._observations.append(value)

    def evidence(self) -> dict[str, list[object]]:
        """What the test recorded, as an entry's `evidence`.

        It holds each kind the test recorded any of.
        """
        kinds = {
            "messages": self._messages,
            "oracle_answers": self._oracle_answers,
            "observations": self._observations,
        }
        return {kind: list(values) for kind, values in kinds.items() if values}


def case_log_lines(
    markers: Sequence[Mapping[str, object]],
    capabilities: Mapping[str, str],
    record: ClientCaseRecord | None,
    *,
    test_passed: bool,
) -> str:
    """The case log's lines for a test that ran.

    One entry per `client_case` marker the test carries.

    Each entry holds the case's capability from `capabilities`, the marker's
    mode, this host's platform and runtime, the checks the test recorded
    followed by `TEST_PASSED`, and the evidence it recorded.

    Raises:
        CaseLogEntryInvalid: An entry fails the bundle's schema.
    """
    checks = (
        [check.model_dump(mode="json") for check in record.checks] if record else []
    )
    checks.append({"name": TEST_PASSED, "passed": test_passed})
    evidence = record.evidence() if record else {}
    lines: list[str] = []
    for marker in markers:
        case = cast("str", marker["id"])
        entry: dict[str, object] = {
            "case": case,
            "capability": capabilities[case],
            "mode": marker["mode"],
            "platform": host_platform().model_dump(mode="json"),
            "runtime": host_runtime(),
            "checks": checks,
            "evidence": evidence,
        }
        checked_case_log_entry(entry)
        lines.append(json.dumps(entry) + "\n")
    return "".join(lines)
