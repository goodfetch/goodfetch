"""Golden scenario 10: offline tests of the crawls of scenarios 05 and 08, run with `pytest` (no network, no model).

- A unit test of scenario 05's book-page handler on a saved page, checking the item and the category it was given.
- A test that runs scenario 05's whole crawl from the Response recording of one earlier live run.
- Two tests of scenario 08's AI extraction: one with a Scripted model reply, one from the Plan learned on the earlier
  run (`plan.frozen`); neither can call a real model.
- Every test runs under a guard that fails on any non-loopback connection, naming the address; the engine itself
  refuses an unrecorded request with `ResponseNotRecordedError`, which names the address too.

The fixtures sit next to this module, made once by a live run: `fixtures/books.warc.gz` (a run started with
`Recording(mode="record", ...)`) and `fixtures/books.plan.json` (scenario 08's Plan file).
"""

import asyncio
import json
import socket
from collections.abc import Iterator
from decimal import Decimal
from pathlib import Path
from typing import Any

import pytest

from goodfetch_foxhound import (
    CrawlRequestFailed,
    CrawlRunReport,
    ModelResponse,
    Recording,
    ReportedTokens,
    seed,
)
from goodfetch_foxhound.testing import (
    AsyncTestEngine,
    Returned,
    SavedResponse,
    ScriptedModel,
)

from scenarios import s05_two_page_crawl as s05
from scenarios import s08_ai_in_crawl_handler as s08

FIXTURES = Path(__file__).parent / "fixtures"
RECORDING = FIXTURES / "books.warc.gz"
LEARNED_PLAN = FIXTURES / "books.plan.json"

# What the earlier live run found: books.toscrape.com holds 1000 books.
EXPECTED_BOOKS = 1000

BOOK_URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"
BOOK_PAGE = """<!DOCTYPE html>
<html lang="en"><head><title>A Light in the Attic | Books to Scrape</title></head>
<body>
<article class="product_page">
  <div class="row">
    <div class="col-sm-6 product_main">
      <h1>A Light in the Attic</h1>
      <p class="price_color">£51.77</p>
      <p class="instock availability"><i class="icon-ok"></i> In stock (22 available)</p>
      <p class="star-rating Three"><i class="icon-star"></i></p>
    </div>
  </div>
</article>
</body></html>
"""


@pytest.fixture(autouse=True)
def no_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    """Fails any test that opens an internet connection from this process, saying which address it tried."""
    real_connect = socket.socket.connect

    def guarded(self: socket.socket, address: Any) -> None:
        if isinstance(address, tuple) and address:
            host = str(address[0])
            if host not in ("127.0.0.1", "::1", "localhost"):
                pytest.fail(f"network access attempted: connect to {address!r}")
        real_connect(self, address)

    monkeypatch.setattr(socket.socket, "connect", guarded)
    yield


def _no_misses(report: CrawlRunReport) -> None:
    """Fails naming each address the recording did not hold, which the run would otherwise have fetched live."""
    assert report.recording is not None, "the run did not replay a recording"
    missed = [f"{miss.method} {miss.url}" for miss in report.recording.misses]
    assert not missed, f"requests reached for the network, not in the recording: {missed}"


def test_book_handler_makes_the_item_with_the_category_it_was_given() -> None:
    async def scenario() -> None:
        async with AsyncTestEngine.start() as engine:
            attempt = await engine.run_handler(
                s05.crawl,
                s05.book,
                url=BOOK_URL,
                payload=s05.BookPayload(category="Poetry"),
                response=SavedResponse(body=BOOK_PAGE),
                tier="browser",
            )
            assert isinstance(attempt.ended, Returned), attempt.ended
            assert list(attempt.records(s05.books)) == [
                s05.Book(
                    title="A Light in the Attic",
                    price=Decimal("51.77"),
                    in_stock=22,
                    rating=3,
                    url=BOOK_URL,
                    category="Poetry",
                )
            ]

    asyncio.run(scenario())


def test_whole_crawl_replays_the_recorded_run() -> None:
    async def scenario() -> None:
        async with AsyncTestEngine.start() as engine:
            run = await engine.connection.start_crawl(
                s05.crawl,
                seeds=s05.seeds,
                recording=Recording(mode="replay", path=RECORDING),
            )
            async with run:
                report = await run.finish()
                failed = [
                    f"{outcome.request.url}: {outcome.error.code}"
                    async for outcome in run.outcomes(kinds=["failed"])
                    if isinstance(outcome, CrawlRequestFailed)
                ]
                stored = [record.value async for record in run.records(s05.books)]
            _no_misses(report)
            assert not failed, f"failed requests: {failed}"
            assert len(stored) == EXPECTED_BOOKS
            assert report.records["books"].committed == EXPECTED_BOOKS
            assert len({book.url for book in stored}) == EXPECTED_BOOKS

    asyncio.run(scenario())


def test_ai_extraction_from_a_scripted_model_reply() -> None:
    reply = ModelResponse(
        text=json.dumps(
            {"title": "A Light in the Attic", "price": "51.77", "in_stock": 22, "rating": 3}
        ),
        finish_reason="stop",
        usage=ReportedTokens(input_tokens=400, output_tokens=40),
    )
    scripted = ScriptedModel([reply])

    async def scenario() -> None:
        # Only the Scripted model is registered, so no real provider can be reached; closing the engine fails if the
        # reply was not used, and a second call would raise ScriptedModelExhaustedError.
        async with AsyncTestEngine.start(
            settings={"ai": {"model": "scripted/books"}, "extract": {"mode": "model"}},
            model_providers={"scripted": scripted},
        ) as engine:
            attempt = await engine.run_handler(
                s08.crawl,
                s08.book,
                url=BOOK_URL,
                response=SavedResponse(body=BOOK_PAGE),
                tier="browser",
            )
            assert isinstance(attempt.ended, Returned), attempt.ended
            assert list(attempt.records(s08.books)) == [
                s08.BookRecord(
                    title="A Light in the Attic",
                    price=Decimal("51.77"),
                    in_stock=22,
                    rating=3,
                    url=BOOK_URL,
                )
            ]
            assert attempt.usage.calls == 1

    asyncio.run(scenario())
    assert len(scripted.requests) == 1
    assert scripted.remaining == 0


def test_ai_crawl_replays_the_learned_plan_without_a_model() -> None:
    async def scenario() -> None:
        # No model provider is registered or configured, and `plan.frozen` forbids a model call and any Plan write:
        # a step the Plan lacks, or a page that drifted, raises instead of reaching a model.
        async with AsyncTestEngine.start() as engine:
            run = await engine.connection.start_crawl(
                s08.crawl,
                seeds=[seed("https://books.toscrape.com/", s08.home)],
                plan=LEARNED_PLAN,
                recording=Recording(mode="replay", path=RECORDING),
                settings={"plan": {"frozen": True}},
            )
            async with run:
                report = await run.finish()
                failed = [
                    f"{outcome.request.url}: {outcome.error.code}"
                    async for outcome in run.outcomes(kinds=["failed"], labels=[s08.book])
                    if isinstance(outcome, CrawlRequestFailed)
                ]
                stored = [record.value async for record in run.records(s08.books)]
            _no_misses(report)
            assert not failed, f"failed requests: {failed}"
            assert stored, "the learned plan produced no book"
            assert report.usage.model.calls == 0

    asyncio.run(scenario())
