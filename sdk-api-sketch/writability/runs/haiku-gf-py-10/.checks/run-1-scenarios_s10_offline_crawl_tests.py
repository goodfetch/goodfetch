"""Offline tests for golden scenarios 05 and 08: no network and no model.

Every test runs on `AsyncTestEngine`, which is offline: a request the test did not record is refused, naming its
address, and a model is reachable only through the Scripted model a test registers. Run them from the working folder
with `python -m pytest scenarios/s10_offline_crawl_tests.py`.

The fixtures named below are files the earlier live run and the saved book page left in `scenarios/fixtures/`.
"""

import asyncio
from decimal import Decimal

from goodfetch_collie_client import ModelResponse, ReportedTokens
from goodfetch_collie_client.testing import ScriptedModel
from goodfetch_foxhound import Recording
from goodfetch_foxhound.testing import AsyncTestEngine, AttemptResult, Failed, Returned, SavedResponse
from scenarios import s05_two_page_crawl as s05
from scenarios import s08_ai_in_crawl_handler as s08

BOOK_URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"
"""A Light in the Attic, the page the saved copy was taken from."""
BOOK_PAGE = "scenarios/fixtures/a_light_in_the_attic.html"
"""A saved copy of that book page, relative to the working directory."""
RECORDING = "scenarios/fixtures/books_live_run.warc"
"""The responses one earlier live run of scenario 05 recorded, as a WARC file."""
EXPECTED_BOOKS = 1000
"""The books the crawl of books.toscrape.com holds: the whole catalogue, as the earlier live run found it."""
AI_MODEL = "scripted/gpt-test"
"""A model reference naming only the Scripted model, so no real provider can answer a call."""
BOOK_REPLY = '{"title": "A Light in the Attic", "price": "51.77", "in_stock": 22, "rating": 3}'


def _assert_returned(result: AttemptResult) -> None:
    ended = result.ended
    if isinstance(ended, Failed):
        error = ended.error
        raise AssertionError(f"the handler failed: {error.code} at {error.url}: {error}")
    assert isinstance(ended, Returned), f"the handler ended {ended!r}, not returned"


async def _book_handler_on_saved_page() -> None:
    async with AsyncTestEngine.start() as engine:
        result = await engine.run_handler(
            s05.crawl,
            s05.book,
            url=BOOK_URL,
            payload=s05.BookPayload(category="Poetry"),
            response=SavedResponse(path=BOOK_PAGE),
        )
    _assert_returned(result)
    [item] = result.records(s05.books)
    assert item == s05.Book(
        title="A Light in the Attic",
        price=Decimal("51.77"),
        in_stock=22,
        rating=3,
        url=BOOK_URL,
        category="Poetry",
    )


def test_book_handler_on_saved_page_yields_the_book_in_its_category() -> None:
    asyncio.run(_book_handler_on_saved_page())


async def _crawl_from_recorded_live_run() -> None:
    async with (
        AsyncTestEngine.start() as engine,
        await engine.connection.start_crawl(
            s05.crawl,
            seeds=s05.seeds,
            recording=Recording(mode="replay", path=RECORDING),
        ) as run,
    ):
        report = await run.finish()
        records = [record async for record in run.records(s05.books)]

    recording = report.recording
    assert recording is not None, "the run did not replay its recording"
    assert not recording.misses, "requests the recording lacks, which would reach the network: " + ", ".join(
        f"{miss.method} {miss.url}" for miss in recording.misses
    )
    assert report.requests.failed == 0, report.requests.by_error_code
    assert len(records) == EXPECTED_BOOKS, f"the dataset holds {len(records)} books, not {EXPECTED_BOOKS}"


def test_crawl_from_recorded_live_run_holds_every_book() -> None:
    asyncio.run(_crawl_from_recorded_live_run())


async def _ai_extraction_from_scripted_reply() -> None:
    scripted = ScriptedModel(
        [
            ModelResponse(
                text=BOOK_REPLY,
                finish_reason="stop",
                usage=ReportedTokens(input_tokens=1200, output_tokens=60),
            )
        ]
    )
    async with AsyncTestEngine.start(
        settings={"ai": {"model": AI_MODEL}},
        model_providers={"scripted": scripted},
    ) as engine:
        result = await engine.run_handler(
            s08.crawl,
            s08.book,
            url=BOOK_URL,
            response=SavedResponse(path=BOOK_PAGE),
        )
    _assert_returned(result)
    assert len(scripted.requests) == 1, f"the model was called {len(scripted.requests)} times, not once"
    assert result.usage.calls == 1, f"the handler reported {result.usage.calls} model calls, not one"
    [record] = result.records(s08.books)
    assert record == s08.BookRecord(
        title="A Light in the Attic",
        price=Decimal("51.77"),
        in_stock=22,
        rating=3,
        url=BOOK_URL,
    )


def test_ai_extraction_uses_only_the_scripted_model_reply() -> None:
    asyncio.run(_ai_extraction_from_scripted_reply())
