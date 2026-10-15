"""Snippet for Testing a user's crawl or automation (#482, ADR 0105), on golden scenario 10.

The crawl of scenario 05 and the AI extraction of scenario 08 (snippet_crawl.py), tested with no network and no model.
Plain `async def` tests and `assert`: pytest with AnyIO's plugin, `unittest.IsolatedAsyncioTestCase` or `anyio.run` run
them alike, and the kit needs no plugin of its own.
"""

from decimal import Decimal

from goodfetch import Recording, ResponseNotRecordedError, seed
from goodfetch import ModelFundsExhaustedError
from goodfetch.testing import AsyncTestEngine, Failed, Returned, SavedResponse, ScriptedModel, Skipped, fake_error
from scenarios.snippet_crawl import BookPayload, ai_book, ai_crawl, book, books, crawl, home

BOOK = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"
RECORDING = Recording(mode="replay", path="tests/books.warc.gz")


# 1. One handler, one saved page: no network, no browser, no Frontier.
async def test_book_page_gives_its_book_with_the_category_passed_to_it() -> None:
    async with AsyncTestEngine.start() as engine:
        attempt = await engine.run_handler(
            crawl,
            book,
            url=BOOK,
            payload=BookPayload(category="Poetry"),
            response=SavedResponse(path="tests/fixtures/a-light-in-the-attic.html"),
        )
    assert attempt.ended == Returned()
    [record] = attempt.records(books)
    assert (record.title, record.price, record.in_stock, record.category) == ("A Light in the Attic", Decimal("51.77"), 22, "Poetry")
    assert attempt.enqueued == []


async def test_book_page_with_no_price_is_skipped() -> None:
    async with AsyncTestEngine.start() as engine:
        attempt = await engine.run_handler(
            crawl, book, url=BOOK, payload=BookPayload(category="Poetry"), response=SavedResponse(body="<h1>No price</h1>")
        )
    assert isinstance(attempt.ended, Skipped) and attempt.ended.reason == "handler"
    assert attempt.ended.message is not None and attempt.ended.message.startswith("no price")


# 2. The whole crawl, from the responses one live run recorded with `Recording(mode="record", path=...)`.
async def test_crawl_collects_every_book_from_the_recording() -> None:
    async with AsyncTestEngine.start() as engine:
        async with await engine.connection.foxhound.start_crawl(crawl, seeds=[seed("https://books.toscrape.com/", home)], recording=RECORDING) as run:
            report = await run.finish()  # no Politeness waits: no request reaches the site
            assert report.requests.failed == 0
            assert len([record async for record in run.records(books)]) == 1000
            # The recording's age is in the report, never a reason to fail.
            assert report.recording is not None and not report.recording.misses


async def test_page_the_recording_lacks_fails_naming_its_address() -> None:
    async with AsyncTestEngine.start() as engine:
        async with await engine.connection.collie.start(recording=RECORDING) as run:
            page = await run.new_page()
            try:
                await page.goto("https://books.toscrape.com/catalogue/page-99.html")
            except ResponseNotRecordedError as error:
                assert error.details.url == "https://books.toscrape.com/catalogue/page-99.html"
            else:
                raise AssertionError("reached a page the recording lacks")


# 3. Scenario 08's AI extraction replays the Plan learned on the live run, committed beside the recording.
async def test_ai_extraction_replays_its_plan_with_no_model_call() -> None:
    async with AsyncTestEngine.start() as engine:  # no model registered: none can be called
        async with await engine.connection.foxhound.start_crawl(
            ai_crawl,
            seeds=[seed(BOOK, ai_book)],
            plan="tests/books.plan.json",
            recording=RECORDING,
            settings={"plan": {"frozen": True}},  # a step the Plan lacks raises PlanStepMissingError, never a model call
        ) as run:
            report = await run.finish()
            assert report.usage.model.calls == 0


# Or a Scripted model: here, a provider whose funds are spent, an error no retry helps.
async def test_handler_fails_cleanly_when_the_model_funds_are_spent() -> None:
    model = ScriptedModel([fake_error(ModelFundsExhaustedError, message="credit balance too low")])
    async with AsyncTestEngine.start(
        model_providers={"scripted": model}, settings={"ai": {"model": "scripted/any"}, "extract": {"mode": "model"}}
    ) as engine:  # closing it fails if a reply is left unused
        attempt = await engine.run_handler(ai_crawl, ai_book, url=BOOK, response=SavedResponse(path="tests/fixtures/a-light-in-the-attic.html"))
    assert isinstance(attempt.ended, Failed) and attempt.ended.error.code == "model_funds_exhausted"
    assert len(model.requests) == 1
