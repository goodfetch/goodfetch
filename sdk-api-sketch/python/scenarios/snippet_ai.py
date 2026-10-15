"""Snippet for Collie's AI verbs and Plans (#473, ADR 0091), on golden scenarios 02, 03, 09 and 11."""

import logging
import re
from collections.abc import Awaitable, Callable
from decimal import Decimal
from typing import Literal

from pydantic import BaseModel, Field

from goodfetch import (
    AsyncGoodfetch,
    AsyncPage,
    AsyncRun,
    BorrowableAsyncPage,
    ExtractionIncompleteError,
    Goodfetch,
    JsonValue,
    PlanDriftError,
    PlanReport,
    expect,
)

log = logging.getLogger("quotes")


async def finish(run: AsyncRun) -> PlanReport:
    """Provisional, #478's: how a program finishes a Collie Run and reads its report."""
    raise NotImplementedError


class Author(BaseModel):
    name: str
    birth_date: str
    birthplace: str


async def author_of_first_love_quote() -> Author:
    """Golden scenario 03, "Repeat AI steps with no model call". The first run compiles each step with the model; a
    later run replays them from quotes.plan.json with no model call, and a step that drifted goes back to the model
    once, alone. In CI, `settings={"plan": {"frozen": True}}` forbids any model call."""
    async with AsyncGoodfetch.connect() as gf, await gf.collie.start(plan="quotes.plan.json") as run:
        page = await run.new_page()
        await page.goto("https://quotes.toscrape.com/")
        for instruction in (
            "open the tag 'love' from the tag list",
            "go to the second page",
            "open the about page of the first quote's author",
        ):
            result = await page.act(instruction)
            if result.step.source == "repair":
                log.warning("step repaired: %s (%s)", result.step.instruction, result.step.drift)

        # A Success check, without a model: the Run is `verified` only if it holds, and only a verified Run promotes
        # a new Plan version (ADR 0009).
        await expect(page).to_have_url(re.compile(r"^https://quotes\.toscrape\.com/author/"))  # a str must equal the URL
        author = (await page.extract("the author's name, birth date and birthplace", Author)).output

        report = await finish(run)
        log.info("plan %s: %s", report.status, report.changed_steps)
        return author


class Book(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)  # pounds, exact
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]
    description: str


async def typed_book(page: AsyncPage) -> Book | None:
    """Golden scenario 02, "Extract a typed object with a chosen model": the model chosen for this call alone (#475
    types it), and its usage next to the result. A field not on the page raises, naming it."""
    await page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html")
    try:
        result = await page.extract("the book on this page", Book, model="ollama/qwen3:8b")
    except ExtractionIncompleteError as error:
        log.warning("fields not on the page: %s (found %s)", error.details.missing, error.details.found)
        return None
    log.info("extracted with %d tokens", result.usage.input_tokens + result.usage.output_tokens)
    return result.output


class Listed(BaseModel):
    title: str
    price: Decimal


async def travel_books(run: AsyncRun, playwright_page: BorrowableAsyncPage) -> list[Listed]:
    """Golden scenario 09, "Add an AI step to an existing Playwright program": the borrowed page gets the verbs, and
    ending the borrow never closes or navigates the program's own page (ADR 0078)."""
    async with await run.borrow(playwright_page) as page:
        return (await page.locator("ol.row").extract("every book listed: title and price", list[Listed])).output


async def search(page: AsyncPage, query: str) -> None:
    """`observe` finds without acting; the program acts on a candidate through its ordinary locator."""
    boxes = (await page.observe("the search box in the header")).output
    if not boxes:
        return  # none is an answer
    await boxes[0].locator.fill(query)
    await boxes[0].locator.press("Enter")


async def search_with_act(page: AsyncPage, query: str) -> None:
    """The same with `act`, one action per call. Values typed into the page travel as variables: the model and the Plan
    see only `%query%`, so one Plan step serves every query."""
    await page.act("type %query% into the search box", variables={"query": query})
    await page.act("submit the search")


async def own_agent_loop(page: AsyncPage, ask_model: Callable[[str], Awaitable[tuple[str, bool]]]) -> None:
    """A program's own agent loop reads the observation and acts on references (Playwright 1.59+'s names)."""
    for _ in range(10):
        observation = await page.aria_snapshot(mode="ai")  # bounded by observation.max_size, page text untrusted
        ref, done = await ask_model(observation)
        if done:
            break
        await page.get_by_ref(ref).click()  # references last as long as their element


async def raw_schema(page: AsyncPage) -> JsonValue:
    """The library-free way down: a hand-written JSON Schema gives a JsonValue to narrow."""
    schema: dict[str, JsonValue] = {"type": "object", "properties": {"price": {"type": "string"}}}
    return (await page.extract("the price", schema)).output


def frozen_replay_sync() -> None:
    """The sync client, with the Plan frozen: a drift that cannot be repaired names the step and why."""
    with Goodfetch.connect() as gf, gf.collie.start(plan="quotes.plan.json", settings={"plan": {"frozen": True}}) as run:
        page = run.new_page()
        page.goto("https://quotes.toscrape.com/")
        try:
            page.act("open the tag 'love' from the tag list")
        except PlanDriftError as error:
            log.error("%s: %s", error.details.instruction, error.details.why_no_repair)
            raise
        expect(page.get_by_text("Viewing tag: love")).to_be_visible()
