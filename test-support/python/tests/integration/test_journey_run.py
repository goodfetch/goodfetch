"""The Python Journey helper (ADR 0063; TESTING.md, Journeys).

A run of a fixture Journey writes the result document that
`goodfetch-conformance read --journey` holds to that Journey's file. These
tests start runs from fixture files, whose IDs no catalog Journey has, so
they carry no `journey` marker and declare no Feature.
"""

import http.client
import json
import os
import re
from collections.abc import Callable, Iterator, Mapping
from pathlib import Path
from typing import TYPE_CHECKING, Any, Protocol, cast
from urllib.parse import urlsplit

import anyio.to_thread
import pytest
from goodfetch_test_support import (
    JourneyRun,
    ProvingGround,
    SurfaceUnlisted,
    TestEnv,
    Tool,
    VariantNotProved,
    proving_ground_routes,
    start_journey_run,
)
from goodfetch_test_support.generated.result_document import (
    JourneyOutcome,
    OracleOutcome,
)

if TYPE_CHECKING:
    from conftest import SampleRun

pytestmark = pytest.mark.anyio

FIXTURES = Path(__file__).parent / "fixtures" / "journey-run"


@pytest.fixture(scope="module")
def anyio_backend() -> str:
    """A Journey variant runs on asyncio alone, as `journey` runs it."""
    return "asyncio"


class StartRun(Protocol):
    """Starts a run of a fixture Journey; the test's end tears it down."""

    def __call__(
        self, name: str, surface: str, environ: Mapping[str, str] | None = None
    ) -> JourneyRun:
        """Starts the variant on `surface` of the fixture `name`."""
        ...


@pytest.fixture
def start_run(request: pytest.FixtureRequest) -> Iterator[StartRun]:
    """Starts runs of fixture Journeys, each closed at the test's end."""
    runs: list[JourneyRun] = []
    # pytest leaves `FixtureRequest.node` unannotated.
    item = cast("pytest.Item", request.node)  # pyright: ignore[reportUnknownMemberType]

    def start(
        name: str, surface: str, environ: Mapping[str, str] | None = None
    ) -> JourneyRun:
        run = start_journey_run(
            FIXTURES / name,
            surface,
            test_name=item.nodeid,
            environ=os.environ if environ is None else environ,
        )
        runs.append(run)
        return run

    yield start
    for run in runs:
        run.close()


def read(run: JourneyRun, document: Path, journey: str) -> int:
    """`goodfetch-conformance read <document> --journey <file>`'s exit code."""
    reader = run.test_env.start(
        [
            run.test_env.tools.path(Tool.CONFORMANCE),
            *("read", document, "--journey", FIXTURES / journey),
        ]
    )
    reader.communicate()
    return reader.returncode


def written(path: Path) -> dict[str, Any]:
    """The document a finished run wrote, read back from its file."""
    return cast("dict[str, Any]", json.loads(path.read_text("utf-8")))


def outcomes(entries: list[dict[str, Any]]) -> list[str]:
    """Each entry of a document's list as `<outcome> <text>`."""
    return [f"{entry['outcome']} {entry['text']}" for entry in entries]


async def ask_version(run: JourneyRun) -> None:
    """Asks the run's executable for its version, failing unless it answers."""

    def ask() -> None:
        process = run.command(["--version"])
        process.communicate()
        if process.returncode != 0:
            message = f"--version exited with {process.returncode}"
            raise AssertionError(message)

    await anyio.to_thread.run_sync(ask)


def exchange(proving_ground: ProvingGround, host: str, path: str) -> str:
    """One `GET` from the Proving ground, naming `host` in `Host`."""
    address, port = proving_ground.address.rsplit(":", 1)
    connection = http.client.HTTPConnection(address, int(port), timeout=30)
    try:
        connection.request("GET", path, headers={"Host": host})
        return connection.getresponse().read().decode("utf-8")
    finally:
        connection.close()


Product = Callable[[str], dict[str, str]]
"""Reads the product a `catalog` product page shows: `catalog_product`."""


def catalog_record(
    proving_ground: ProvingGround | None, product: Product
) -> dict[str, str]:
    """The product of `catalog/2`'s start page, read as any tool would read it."""
    assert proving_ground is not None
    listing = cast(
        "dict[str, Any]",
        json.loads(exchange(proving_ground, "tasks", proving_ground_routes().tasks)),
    )
    entry = next(entry for entry in listing["tasks"] if entry["id"] == "catalog/2")
    url = urlsplit(cast("str", entry["start_url"]))
    return product(exchange(proving_ground, url.netloc, url.path))


async def read_product(run: JourneyRun, product: Product) -> dict[str, str]:
    """The Record of `catalog/2`, read off the event loop."""
    return await anyio.to_thread.run_sync(catalog_record, run.proving_ground, product)


async def test_a_run_whose_steps_pass_and_task_is_verified_writes_a_proved_document(
    start_run: StartRun, catalog_product: Product
) -> None:
    """The document is `proved`, and the reader accepts it against its Journey."""
    run = start_run("J-961.toml", "sdk-python")
    await run.step("Ask the executable for its version", lambda: ask_version(run))
    record = await run.step(
        "Read the product its task's start page shows",
        lambda: read_product(run, catalog_product),
    )
    await run.expect_task(
        "The oracle verifies the product's Record",
        {"status": "success", "records": [record]},
    )

    finished = run.finish()

    assert finished.outcome == JourneyOutcome.proved
    assert finished.failures == []
    finished.assert_proved()
    assert read(run, finished.path, "J-961.toml") == 0


async def test_a_skipped_step_gives_a_document_the_reader_refuses(
    start_run: StartRun,
) -> None:
    """The step called early does not run, and the reader refuses the order."""
    run = start_run("J-960.toml", "sdk-python")
    ran = False

    async def once_more() -> None:
        nonlocal ran
        ran = True

    await run.step("Ask the executable for its version", lambda: ask_version(run))
    await run.step("Ask the executable for its version once more", once_more)

    finished = run.finish()

    assert not ran
    assert outcomes(written(finished.path)["steps"]) == [
        "passed Ask the executable for its version",
        "manual Read the version aloud",
        "failed Ask the executable for its version once more",
        "not_reached Ask the executable for its help",
    ]
    assert finished.failures == [
        "step `Ask the executable for its version once more` is called before "
        "step 3 `Ask the executable for its help`"
    ]
    assert read(run, finished.path, "J-960.toml") == 1


async def test_a_false_success_answer_gives_a_failed_document_the_reader_accepts(
    start_run: StartRun, catalog_product: Product
) -> None:
    """The check fails, the task keeps the answer, and `assert_proved` raises."""
    run = start_run("J-961.toml", "sdk-python")
    await run.step("Ask the executable for its version", lambda: ask_version(run))
    record = await run.step(
        "Read the product its task's start page shows",
        lambda: read_product(run, catalog_product),
    )
    assert record is not None
    answer = await run.expect_task(
        "The oracle verifies the product's Record",
        {"status": "success", "records": [{**record, "price": "0.01"}]},
    )

    finished = run.finish()

    assert answer is not None
    assert answer.outcome == OracleOutcome.false_success
    assert finished.outcome == JourneyOutcome.failed
    document = written(finished.path)
    assert [(check["task"], check["outcome"]) for check in document["checks"]] == [
        ("catalog/2", "failed")
    ]
    assert [(task["id"], task["answer"]["outcome"]) for task in document["tasks"]] == [
        ("catalog/2", "false_success")
    ]
    assert finished.failures == [
        "the oracle answers `false_success` for task `catalog/2` of check "
        "`The oracle verifies the product's Record`"
    ]
    with pytest.raises(
        VariantNotProved,
        match=re.escape("Journey variant J-961@sdk-python is `failed`, not `proved`"),
    ):
        finished.assert_proved()
    assert read(run, finished.path, "J-961.toml") == 0


async def test_a_journey_with_no_proving_ground_table_starts_none(
    start_run: StartRun,
) -> None:
    """Neither the home nor the document has a Proving ground."""
    run = start_run("J-960.toml", "sdk-python")

    finished = run.finish()

    assert run.test_env.proving_ground is None
    assert run.proving_ground is None
    assert "proving_ground" not in written(finished.path)


async def test_the_proving_ground_starts_on_the_journeys_seed_whatever_the_test_seed(
    start_run: StartRun,
) -> None:
    """`GOODFETCH_TEST_SEED` sets the test's seed, never the Journey's."""
    run = start_run(
        "J-961.toml", "sdk-python", {**os.environ, "GOODFETCH_TEST_SEED": "4242"}
    )

    finished = run.finish()

    assert run.test_env.seed == 4242
    assert run.proving_ground is not None
    assert run.proving_ground.seed == 961
    assert written(finished.path)["proving_ground"]["seed"] == 961


async def test_the_steps_after_a_failed_step_are_not_reached_and_a_manual_step_manual(
    start_run: StartRun,
) -> None:
    """Neither a later step nor a check runs; the reader accepts the document."""
    run = start_run("J-960.toml", "sdk-python")
    ran = False

    async def fails() -> None:
        message = "no version"
        raise RuntimeError(message)

    async def runs() -> None:
        nonlocal ran
        ran = True

    await run.step("Ask the executable for its version", fails)
    await run.step("Ask the executable for its help", runs)
    await run.check("The version names the Product", runs)

    finished = run.finish()

    assert not ran
    document = written(finished.path)
    assert outcomes(document["steps"]) == [
        "failed Ask the executable for its version",
        "manual Read the version aloud",
        "not_reached Ask the executable for its help",
        "not_reached Ask the executable for its version once more",
    ]
    assert outcomes(document["checks"]) == ["not_reached The version names the Product"]
    assert finished.failures == [
        "step 1 `Ask the executable for its version` failed: no version"
    ]
    assert read(run, finished.path, "J-960.toml") == 0


async def test_a_check_of_another_variants_surface_is_neither_run_nor_recorded(
    start_run: StartRun,
) -> None:
    """Calling it is a misuse, recorded as a check the file does not hold."""
    run = start_run("J-960.toml", "sdk-python")
    ran = False

    async def runs() -> None:
        nonlocal ran
        ran = True

    await run.check("The CLI prints the version on one line", runs)
    finished = run.finish()

    assert not ran
    assert outcomes(written(finished.path)["checks"]) == [
        "not_reached The version names the Product",
        "failed The CLI prints the version on one line",
    ]
    assert finished.failures == [
        "check `The CLI prints the version on one line` is not a check of J-960 "
        "that names no task"
    ]


async def test_each_check_naming_one_task_is_judged_on_its_own_submission(
    start_run: StartRun, catalog_product: Product
) -> None:
    """The task is listed once per check, each with its own answer."""
    run = start_run("J-962.toml", "sdk-python")
    record = await run.step(
        "Read the product its task's start page shows, twice",
        lambda: read_product(run, catalog_product),
    )
    assert record is not None
    await run.expect_task(
        "The oracle verifies the first read's Record",
        {"status": "success", "records": [record]},
    )
    await run.expect_task(
        "The oracle verifies the second read's Record",
        {"status": "success", "records": [{**record, "price": "0.01"}]},
    )

    finished = run.finish()

    assert [
        (task["id"], task["answer"]["outcome"])
        for task in written(finished.path)["tasks"]
    ] == [("catalog/2", "verified"), ("catalog/2", "false_success")]
    assert read(run, finished.path, "J-962.toml") == 0


def test_a_run_refuses_a_surface_its_journey_does_not_list(
    start_run: StartRun,
) -> None:
    """The refusal names the surfaces the Journey lists."""
    with pytest.raises(
        SurfaceUnlisted,
        match=re.escape("J-961 lists no surface `cli`: it lists sdk-python"),
    ):
        start_run("J-961.toml", "cli")


DECLARED = """
from goodfetch_test_support import JourneyRun, journey


@journey("J-002", "sdk-python")
async def test_j002(run: JourneyRun) -> None:
    pass
"""
"""A catalog Journey's variant whose body takes no step."""


def test_journey_declares_a_test_carrying_its_variant_as_a_marker(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """The marker listing gives the variant, which `-m` selects."""
    listing = test_env.folder / "markers.json"

    outcome = sample_run.run(
        DECLARED, args=["--collect-only", f"--marker-listing={listing}"]
    )

    assert outcome.status == 0, outcome.output
    assert json.loads(listing.read_text("utf-8"))["tests"] == [
        {
            "id": "tests/test_sample.py::test_j002",
            "journey": [{"id": "J-002", "surface": "sdk-python"}],
            "covers": [],
            "client_case": [],
            "skipped": False,
        }
    ]


def test_journey_fails_its_test_when_the_variant_is_not_proved(
    sample_run: "SampleRun",
) -> None:
    """A body that takes no step leaves every step not reached."""
    outcome = sample_run.run(DECLARED)

    assert outcome.results == {"test_j002": "failure"}, outcome.output
    assert "Journey variant J-002@sdk-python is `failed`, not `proved`" in (
        outcome.output
    )
