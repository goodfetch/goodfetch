"""Golden scenario 10: the crawl of scenario 05 and the AI extraction of scenario 08, tested with no network and no
model, on pytest with AnyIO's plugin. The kit needs no plugin of its own. Every Run the test engine starts replays a
recording, an empty one when it names none, so a request it lacks is never sent: it fails with
`response_not_recorded`, naming its address. Run it with `pytest scenarios/s10_offline_crawl_tests.py`.
"""

from collections.abc import AsyncIterator
from decimal import Decimal

import pytest
from pydantic import AnyUrl

from goodfetch_foxhound import Recording, seed
from goodfetch_foxhound.testing import AsyncTestEngine, Returned, SavedResponse
from scenarios import s08_ai_in_crawl_handler as ai
from scenarios.s05_two_page_crawl import Book, BookPayload, book, books, crawl, seeds

pytestmark = pytest.mark.anyio

BOOK = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"
# One live run of scenario 05 recorded these, with `recording=Recording(mode="record", path=...)` on its start.
RECORDING = Recording(mode="replay", path="tests/books.warc.gz")


@pytest.fixture
async def engine() -> AsyncIterator[AsyncTestEngine]:
    async with AsyncTestEngine.start() as engine:
        yield engine


async def test_book_page_gives_its_book_with_the_category_passed_to_it(
    engine: AsyncTestEngine,
) -> None:
    # The handler alone, on a saved page: no Frontier, no browser on the `http` tier.
    attempt = await engine.run_handler(
        crawl,
        book,
        url=BOOK,
        payload=BookPayload(category="Poetry"),
        response=SavedResponse(path="tests/fixtures/a-light-in-the-attic.html"),
    )
    assert attempt.ended == Returned()
    assert list(attempt.records(books)) == [
        Book(
            title="A Light in the Attic",
            price=Decimal("51.77"),
            in_stock=22,
            rating=3,
            url=AnyUrl(BOOK),
            category="Poetry",
        )
    ]


async def test_crawl_collects_every_book_from_the_recording(
    engine: AsyncTestEngine,
) -> None:
    async with await engine.connection.start_crawl(
        crawl, seeds=seeds, recording=RECORDING
    ) as run:
        report = await run.finish()  # no Politeness waits: nothing reaches the site
    assert report.recording is not None
    assert list(report.recording.misses) == []  # a miss shows its method and URL here
    assert report.records["books"].committed == 1000


async def test_ai_extraction_replays_its_plan_with_no_model_call(
    engine: AsyncTestEngine,
) -> None:
    # The engine has no Model provider: a model call would fail with `model_not_configured`.
    async with await engine.connection.start_crawl(
        ai.crawl,
        seeds=[seed(BOOK, ai.book)],
        plan=ai.PLAN,  # what scenario 08's live run learned, committed
        recording=RECORDING,
        # A step the Plan lacks fails with `plan_step_missing` instead of calling a model.
        settings={"plan": {"frozen": True}},
    ) as run:
        report = await run.finish()
    assert report.recording is not None
    assert list(report.recording.misses) == []  # each miss names its URL
    assert report.requests.failed == 0
    assert report.records["books"].committed == 1
    assert report.usage.model.calls == 0
