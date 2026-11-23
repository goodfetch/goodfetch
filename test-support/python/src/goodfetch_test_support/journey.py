"""The Python Journey helper (ADR 0063; TESTING.md, Journeys).

One variant run of a Journey of `journeys/`, from its start to the `journey`
result document it writes, by the rules of the Rust `JourneyRun`. A run
reads the Journey file with `tomllib`, validated against
`journeys/journey.schema.json`, starts the test's home through
`goodfetch-test-env` with the Journey's own Proving ground seed (never the
test's), asks the Product's executable for its version, records each step
and check in the file's order and wording, and writes the document through
`write_journey_document`. The helper declares no Feature: a Journey test's
Features are its Journey's `covers`.
"""

import http
import http.client
import json
import os
import re
import subprocess
import tomllib
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType
from typing import Self, TypeVar, cast

import anyio
import anyio.to_thread
import pytest

from goodfetch_test_support.case_log import host_platform, host_runtime
from goodfetch_test_support.environment import Product, TestEnv
from goodfetch_test_support.generated.result_document import (
    CheckOutcome,
    CheckResult,
    Executable,
    JourneyId,
    JourneyOutcome,
    OracleAnswer,
    OracleOutcome,
    ProvingGroundCopy,
    StepOutcome,
    StepResult,
    SurfacePackage,
    TaskId,
    TaskResult,
)
from goodfetch_test_support.generated.result_document import (
    Product as DocumentProduct,
)
from goodfetch_test_support.proving_ground import ProvingGround, proving_ground_routes
from goodfetch_test_support.result_document import write_journey_document
from goodfetch_test_support.schemas import SchemaError, validate

CATALOG = Path(__file__).resolve().parents[4] / "journeys"
"""The catalog's folder, `journeys/` at the repository's root."""

RESULT_DOCUMENTS_VARIABLE = "GOODFETCH_TEST_RESULT_DOCUMENTS"
"""The folder each variant run also writes its document to, which `just test` sets."""

_VERSION = re.compile(r"goodfetch-(collie|foxhound) (\S+)")
"""What an executable's `--version` prints: `goodfetch-<product> <version>`."""

_T = TypeVar("_T")


class JourneyUnreadable(Exception):
    """The Journey file cannot be read."""


class JourneyInvalid(Exception):
    """The Journey file is not TOML, or fails `journeys/journey.schema.json`."""


class JourneyOther(Exception):
    """`journeys/<id>.toml` holds another Journey than `<id>`."""


class SurfaceUnlisted(Exception):
    """The Journey lists no variant on the surface asked for."""


class EngineVersionUnreported(Exception):
    """The executable's `--version` does not report its Product and version."""


class OracleRefused(Exception):
    """The Proving ground's oracle gave no answer to a submission."""


class VariantNotProved(Exception):
    """A finished variant's document is not `proved`, or the run recorded a failure."""


@dataclass(frozen=True)
class _Step:
    text: str
    manual: bool


@dataclass(frozen=True)
class _Check:
    text: str
    task: str | None


@dataclass(frozen=True)
class _Journey:
    """What the helper reads of a Journey file; the schema holds the rest."""

    id: str
    product: Product
    seed: int | None
    """The Proving ground's seed, for a Journey with a `[proving_ground]` table."""
    tasks: tuple[str, ...]
    surfaces: tuple[str, ...]
    steps: tuple[_Step, ...]
    checks: tuple[_Check, ...]
    """The variant's own checks: those scoped to other surfaces are left out."""


def _read_journey(file: Path, surface: str) -> _Journey:
    try:
        text = file.read_text("utf-8")
    except OSError as error:
        message = f"the Journey file {file} cannot be read"
        raise JourneyUnreadable(message) from error
    try:
        value = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        message = f"the Journey file {file} is not TOML"
        raise JourneyInvalid(message) from error
    try:
        validate(value, CATALOG / "journey.schema.json")
    except SchemaError as error:
        message = f"the Journey file {file} fails journeys/journey.schema.json"
        raise JourneyInvalid(message) from error
    proving_ground = cast("dict[str, object] | None", value.get("proving_ground"))
    tasks = cast(
        "list[dict[str, str]]",
        [] if proving_ground is None else proving_ground.get("tasks", []),
    )
    steps = cast("list[dict[str, str]]", value["steps"])
    checks = cast("list[dict[str, object]]", value["expect"])
    return _Journey(
        id=cast("str", value["id"]),
        product=Product(cast("str", value["product"])),
        seed=None if proving_ground is None else cast("int", proving_ground["seed"]),
        tasks=tuple(task["id"] for task in tasks),
        surfaces=tuple(cast("dict[str, str]", value["surfaces"])),
        steps=tuple(_Step(step["text"], "manual" in step) for step in steps),
        checks=tuple(
            _Check(cast("str", check["text"]), cast("str | None", check.get("task")))
            for check in checks
            if surface in cast("list[str]", check.get("surfaces") or [surface])
        ),
    )


def _reported_version(test_env: TestEnv, engine: Path) -> Executable:
    """The Product and version `engine --version` reports."""
    try:
        process = test_env.start([engine, "--version"])
        stdout, _ = process.communicate()
    except OSError as error:
        message = f"{engine} --version cannot start: {error}"
        raise EngineVersionUnreported(message) from error
    printed = stdout.decode(errors="replace").strip()
    found = _VERSION.fullmatch(printed)
    if process.returncode != 0 or found is None:
        message = (
            f"{engine} --version does not report its Product and version "
            f"(code {process.returncode}): {printed}"
        )
        raise EngineVersionUnreported(message)
    return Executable(product=DocumentProduct(found[1]), version=found[2])


def _judge(
    proving_ground: ProvingGround, submission: Mapping[str, object]
) -> OracleAnswer:
    """The oracle's answer to `submission`, checked against its schema first."""
    validate(submission, "proving-ground-oracle-submission.schema.json")
    host, port = proving_ground.address.rsplit(":", 1)
    connection = http.client.HTTPConnection(host, int(port), timeout=30)
    try:
        connection.request(
            "POST",
            proving_ground_routes().oracle,
            json.dumps(submission),
            {"Content-Type": "application/json"},
        )
        response = connection.getresponse()
        body = response.read().decode("utf-8", errors="replace")
    finally:
        connection.close()
    if response.status != http.HTTPStatus.OK:
        message = f"the Proving ground's oracle answered {response.status}: {body}"
        raise OracleRefused(message)
    answer: object = json.loads(body)
    if not isinstance(answer, dict) or not {"outcome", "task"} <= answer.keys():
        message = "the Proving ground's oracle answered no outcome"
        raise OracleRefused(message)
    return OracleAnswer.model_validate(answer)


def judge_task(
    proving_ground: ProvingGround, task: str, records: Sequence[object]
) -> OracleAnswer:
    """The oracle's answer to `records`, a tool's result for the task `task`; it blocks.

    The result is a success's. TypeScript's `judgeTask` is its twin.

    Raises:
        OracleRefused: The oracle gave no answer.
    """
    return _judge(
        proving_ground, {"task": task, "status": "success", "records": list(records)}
    )


def _failure_of(error: Exception) -> str:
    return str(error) or repr(error)


@dataclass(frozen=True)
class FinishedJourneyRun:
    """A finished run: the document written and what failed."""

    variant: str
    """The variant, `<id>@<surface>`."""
    path: Path
    """Where the document was written in the test's folder."""
    outcome: JourneyOutcome
    """The variant's outcome as its document states it."""
    failures: list[str]
    """Each failure the run recorded: a failed step or check, an answer that
    is not `verified`, or a misuse."""

    def assert_proved(self) -> None:
        """Fails unless the document states `proved` and nothing failed.

        Raises:
            VariantNotProved: The document is `failed`, or a failure was recorded.
        """
        if self.outcome == JourneyOutcome.proved and not self.failures:
            return
        listed = "".join(f"\n- {failure}" for failure in self.failures)
        message = (
            f"Journey variant {self.variant} is `{self.outcome}`, not `proved` "
            f"({self.path}):{listed}"
        )
        raise VariantNotProved(message)


class JourneyRun:
    """One variant run of a Journey.

    A test awaits `step` for each step of the file, in order, then `check`
    and `expect_task` for its checks, then calls `finish`; `close`, or the
    end of a `with` block, tears the home down. `journey` does all but the
    steps and checks.
    """

    def __init__(
        self,
        journey: _Journey,
        *,
        surface: str,
        test_env: TestEnv,
        engine: Path,
        executable: Executable,
        result_documents: Path | None,
    ) -> None:
        """Takes a started run; `start_journey_run` makes one."""
        self.id = journey.id
        """The Journey's ID."""
        self.surface = surface
        """The variant's surface."""
        self.product = journey.product
        """The Journey's Product."""
        self.engine = engine
        """The executable the variant drives: the manifest's entry for the Product."""
        self.test_env = test_env
        """The test's home, as `goodfetch-test-env` describes it."""
        self.env: Mapping[str, str] = {
            **test_env.env,
            journey.product.engine_path_variable: str(engine),
        }
        """The home's environment with the Product's engine path: what the SDK takes."""
        self.proving_ground = None if journey.seed is None else test_env.proving_ground
        """The Proving ground on the Journey's seed, when the file has the table."""
        self._journey = journey
        self._executable = executable
        self._result_documents = result_documents
        self._steps: list[StepResult | None] = []
        self._taken = [False for _ in journey.steps]
        self._checks: list[CheckResult | None] = [None for _ in journey.checks]
        self._extra_checks: list[CheckResult] = []
        self._answers: dict[int, OracleAnswer] = {}
        self._failures: list[str] = []
        self._surface_package: SurfacePackage | None = None
        self._running = 0

    def __enter__(self) -> Self:
        """Gives the run, which the block's end closes."""
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        """Closes the run, keeping the test's folder when the block raised."""
        self.close(failed=kind is not None)

    def command(
        self, args: Sequence[str | os.PathLike[str]]
    ) -> subprocess.Popen[bytes]:
        """Starts the executable with `args` in the home: an `on = "cli"` step."""
        return self.test_env.start([self.engine, *args])

    def surface_package(self, name: str, version: str) -> None:
        """Records the surface's package: the variant's wheel and its version."""
        self._surface_package = SurfacePackage(name=name, version=version)

    async def step(self, wording: str, body: Callable[[], Awaitable[_T]]) -> _T | None:
        """Runs the step worded `wording` when it is next and nothing failed.

        It records the step's outcome, `failed` when `body` raises, and gives
        what `body` returned, or `None` when it did not run or failed.
        """
        steps = self._journey.steps
        found = self._untaken_step(wording)
        manual_reached = found is not None and all(
            self._taken[index] or steps[index].manual for index in range(found + 1)
        )
        self._take_manual_steps()
        if manual_reached:
            # A manual step the run reached: recorded `manual` just above.
            return None
        if found is None:
            again = any(step.text == wording for step in steps)
            self._misused_step(
                wording,
                "is called a second time" if again else f"is not a step of {self.id}",
            )
            return None
        following = self._taken.index(False)
        if found != following:
            self._taken[found] = True
            self._misused_step(
                wording,
                f"is called before step {following + 1} `{steps[following].text}`",
            )
            return None
        self._taken[found] = True
        if self._failures:
            self._steps.append(
                StepResult(text=wording, outcome=StepOutcome.not_reached)
            )
            return None
        # The step's place is kept while its body runs, so the steps stay in
        # the order they were called.
        place = len(self._steps)
        self._steps.append(None)
        self._running += 1
        try:
            value = await body()
        except Exception as error:  # a step fails on any error.
            self._failures.append(
                f"step {found + 1} `{wording}` failed: {_failure_of(error)}"
            )
            self._steps[place] = StepResult(text=wording, outcome=StepOutcome.failed)
            return None
        finally:
            self._running -= 1
        self._steps[place] = StepResult(text=wording, outcome=StepOutcome.passed)
        return value

    async def check(self, wording: str, body: Callable[[], Awaitable[object]]) -> None:
        """Runs the check worded `wording`, one naming no task, when nothing failed.

        It records the check's outcome: `failed` when `body` raises.
        """
        index = next(
            (
                place
                for place, check in enumerate(self._journey.checks)
                if check.text == wording and check.task is None
            ),
            None,
        )
        if index is None or self._checks[index] is not None:
            misuse = (
                f"is not a check of {self.id} that names no task"
                if index is None
                else "is called a second time"
            )
            self._failures.append(f"check `{wording}` {misuse}")
            self._extra_checks.append(
                CheckResult(text=wording, outcome=CheckOutcome.failed)
            )
            return
        self._checks[index] = CheckResult(
            text=wording, outcome=CheckOutcome.not_reached
        )
        if self._failures:
            return
        self._running += 1
        try:
            await body()
        except Exception as error:  # a check fails on any error.
            self._failures.append(f"check `{wording}` failed: {_failure_of(error)}")
            self._checks[index] = CheckResult(text=wording, outcome=CheckOutcome.failed)
            return
        finally:
            self._running -= 1
        self._checks[index] = CheckResult(text=wording, outcome=CheckOutcome.passed)

    async def expect_task(
        self, check: str, submission: Mapping[str, object]
    ) -> OracleAnswer | None:
        """Submits the tool's result for the check worded `check` to the oracle.

        The check names a task; `submission` is the tool's result without it:
        its `status` and its `records` or `files`. Each check naming a task is
        judged on its own submission, when nothing failed before, and recorded
        `passed` when the answer is `verified` and `failed` otherwise.

        Returns:
            The oracle's answer, or `None` when the check was not judged.
        """
        found = next(
            (
                (place, entry.task)
                for place, entry in enumerate(self._journey.checks)
                if entry.text == check and entry.task is not None
            ),
            None,
        )
        if found is None or self._checks[found[0]] is not None:
            misuse = (
                f"is not a check of {self.id} that names a task"
                if found is None
                else "is judged a second time"
            )
            self._failures.append(f"check `{check}` {misuse}")
            self._extra_checks.append(
                CheckResult(text=check, outcome=CheckOutcome.failed)
            )
            return None
        index, task = found
        if self._failures:
            return None
        if self.proving_ground is None:
            self._failures.append(
                f"check `{check}`, task `{task}`: the Journey starts no Proving ground"
            )
            return None
        self._running += 1
        try:
            answer = await anyio.to_thread.run_sync(
                _judge, self.proving_ground, {**submission, "task": task}
            )
        except Exception as error:  # recorded as the check's failure.
            self._failures.append(
                f"check `{check}`, task `{task}`: {_failure_of(error)}"
            )
            return None
        finally:
            self._running -= 1
        verified = answer.outcome == OracleOutcome.verified
        if not verified:
            self._failures.append(
                f"the oracle answers `{answer.outcome}` for task `{task}` of check "
                f"`{check}`"
            )
        self._checks[index] = CheckResult(
            text=check,
            task=TaskId(task),
            outcome=CheckOutcome.passed if verified else CheckOutcome.failed,
        )
        self._answers[index] = answer
        return answer

    def finish(self) -> FinishedJourneyRun:
        """Records what was not reached and writes the document.

        It writes `<id>@<surface>.json` into the test's folder, and into the
        folder `GOODFETCH_TEST_RESULT_DOCUMENTS` names when it is set.
        """
        if self._running > 0:
            self._failures.append(
                "the run finishes while a step or check still runs: await each one"
            )
        self._take_manual_steps()
        for index, step in enumerate(self._journey.steps):
            if not self._taken[index]:
                self._taken[index] = True
                self._steps.append(
                    StepResult(
                        text=step.text,
                        outcome=StepOutcome.manual
                        if step.manual
                        else StepOutcome.not_reached,
                    )
                )
        variant = f"{self.id}@{self.surface}"
        path = self.test_env.folder / f"{variant}.json"
        outcome = self._write(path)
        # The test's own copy is the one returned: in the suite's folder,
        # another test running the same variant may write after it.
        if self._result_documents is not None:
            self._result_documents.mkdir(parents=True, exist_ok=True)
            self._write(self._result_documents / path.name)
        return FinishedJourneyRun(variant, path, outcome, list(self._failures))

    def close(self, *, failed: bool = False) -> None:
        """Tears the test's home down, keeping its folder when `failed` is set."""
        self.test_env.close(failed=failed)

    def _untaken_step(self, wording: str) -> int | None:
        """The first step of the file worded `wording` not recorded yet."""
        return next(
            (
                index
                for index, step in enumerate(self._journey.steps)
                if step.text == wording and not self._taken[index]
            ),
            None,
        )

    def _take_manual_steps(self) -> None:
        """Records each manual step reached: those before the next automated one."""
        for index, step in enumerate(self._journey.steps):
            if self._taken[index]:
                continue
            if not step.manual:
                return
            self._taken[index] = True
            self._steps.append(StepResult(text=step.text, outcome=StepOutcome.manual))

    def _misused_step(self, wording: str, misuse: str) -> None:
        self._failures.append(f"step `{wording}` {misuse}")
        self._steps.append(StepResult(text=wording, outcome=StepOutcome.failed))

    def _write(self, path: Path) -> JourneyOutcome:
        """Writes what the run recorded as the document at `path`."""
        journey = self._journey
        checks = [
            recorded
            or (
                CheckResult(text=check.text, outcome=CheckOutcome.not_reached)
                if check.task is None
                else CheckResult(
                    text=check.text,
                    task=TaskId(check.task),
                    outcome=CheckOutcome.not_reached,
                )
            )
            for check, recorded in zip(journey.checks, self._checks, strict=True)
        ]
        # Each named task, once per check naming it with that check's answer
        # in the checks' order, or once when no check names it.
        tasks: list[TaskResult] = []
        for task in journey.tasks:
            judged = [
                TaskResult(id=TaskId(task))
                if index not in self._answers
                else TaskResult(id=TaskId(task), answer=self._answers[index])
                for index, check in enumerate(journey.checks)
                if check.task == task
            ]
            tasks.extend(judged or [TaskResult(id=TaskId(task))])
        proving_ground = self.proving_ground
        return write_journey_document(
            path,
            journey_id=JourneyId(self.id),
            surface=self.surface,
            executable=self._executable,
            platform=host_platform(),
            runtime=host_runtime(),
            steps=[step for step in self._steps if step is not None],
            checks=[*checks, *self._extra_checks],
            tasks=tasks,
            surface_package=self._surface_package,
            proving_ground=None
            if proving_ground is None
            else ProvingGroundCopy(
                version=proving_ground.version, seed=proving_ground.seed
            ),
        )


def start_journey_run(
    file: Path, surface: str, *, test_name: str, environ: Mapping[str, str]
) -> JourneyRun:
    """Starts the variant on `surface` of the Journey `file` holds.

    Args:
        file: The Journey file: a catalog file, or a fixture in test
            support's own tests.
        surface: The variant's surface, one the Journey lists.
        test_name: The test's full name, from which its seed and folder come.
        environ: The runner's environment, which names the tool manifest and
            may name `GOODFETCH_TEST_RESULT_DOCUMENTS`.

    Raises:
        JourneyUnreadable: The file cannot be read.
        JourneyInvalid: The file is not TOML or fails the Journey schema.
        SurfaceUnlisted: The Journey lists no such surface.
        EngineVersionUnreported: The executable does not report its Product
            and version.
    """
    journey = _read_journey(file, surface)
    if surface not in journey.surfaces:
        message = (
            f"{journey.id} lists no surface `{surface}`: it lists "
            f"{', '.join(journey.surfaces)}"
        )
        raise SurfaceUnlisted(message)
    # The Journey's own seed, never the test's (ADR 0063).
    test_env = TestEnv.open(test_name, environ, proving_ground_seed=journey.seed)
    try:
        engine = test_env.tools.path(journey.product.tool)
        executable = _reported_version(test_env, engine)
    except BaseException:
        test_env.close(failed=True)
        raise
    result_documents = environ.get(RESULT_DOCUMENTS_VARIABLE)
    return JourneyRun(
        journey,
        surface=surface,
        test_env=test_env,
        engine=engine,
        executable=executable,
        result_documents=Path(result_documents) if result_documents else None,
    )


def journey(
    journey_id: str, surface: str
) -> Callable[
    [Callable[[JourneyRun], Awaitable[None]]], Callable[[pytest.FixtureRequest], None]
]:
    """Declares the pytest test of the variant of `journeys/<id>.toml` on `surface`.

    The test carries `journey(id=…, surface=…)`. It starts the run, gives it
    to the decorated `async def` to take the steps and checks on asyncio,
    finishes it and fails unless it is proved. A raise outside a step or
    check fails the test with no document, as the Rust helper's panic does.
    """

    def declare(
        body: Callable[[JourneyRun], Awaitable[None]],
    ) -> Callable[[pytest.FixtureRequest], None]:
        def test(request: pytest.FixtureRequest) -> None:
            # pytest leaves `FixtureRequest.node` unannotated.
            item = cast("pytest.Item", request.node)  # pyright: ignore[reportUnknownMemberType]
            with start_journey_run(
                CATALOG / f"{journey_id}.toml",
                surface,
                test_name=item.nodeid,
                environ=os.environ,
            ) as run:
                if run.id != journey_id:
                    message = (
                        f"journeys/{journey_id}.toml holds {run.id}, not {journey_id}"
                    )
                    raise JourneyOther(message)
                anyio.run(body, run, backend="asyncio")
                run.finish().assert_proved()

        # Copied by hand: `functools.wraps` sets `__wrapped__`, whose
        # signature pytest would read for the test's fixtures.
        test.__name__ = getattr(body, "__name__", test.__name__)
        test.__qualname__ = getattr(body, "__qualname__", test.__qualname__)
        test.__module__ = body.__module__
        test.__doc__ = body.__doc__
        return pytest.mark.journey(id=journey_id, surface=surface)(test)

    return declare
