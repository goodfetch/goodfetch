"""Crawl the first three categories of books.toscrape.com and every book they list, extracting each book's fields
with a language model, under a spending allowance.

- Home page: the first three categories of the side list are enqueued.
- Category pages: every book is enqueued, and the "next" page until there is none.
- Book pages: `page.extract` in `compiled` mode. The model writes the field locators once, kept in the Run's Plan; every
  later book of the same layout is read by replaying the Plan with no model call, and a page that no longer matches is
  repaired once by the model (Drift).
- `ai.allowance.price` caps the model spend of the Run. Once it is reached, every call that needs the model is refused
  with `ModelAllowanceReachedError`, which ends that book's request `skipped` (`model_allowance`) while the crawl goes
  on; steps replayed from the Plan need no model and keep extracting.
- The report lists the books left unextracted with the reason, and the tokens and cost spent.
"""

import asyncio
import os
from collections import Counter
from dataclasses import dataclass
from decimal import Decimal

from pydantic import BaseModel, Field

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncCrawlRun,
    AsyncFoxhound,
    AsyncPage,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestSkipped,
    CrawlRunReport,
    CrawlRunSettings,
    Dataset,
    Label,
    Money,
    StepResult,
    seed,
)

START_URL = "https://books.toscrape.com/"
CATEGORY_COUNT = 3
DEFAULT_ALLOWANCE_USD = "0.50"
DEFAULT_MODEL = "vercel/anthropic/claude-sonnet-4.5"

# One instruction per page type: the Plan keys a compiled step by it, so later books reuse what the first one taught.
BOOK_INSTRUCTION = (
    "Extract the book shown on this product page: its title, its price in pounds sterling as a number without the "
    "currency symbol, the number of copies in stock (from the availability line), and its star rating from 1 to 5 "
    "(the star-rating element's class names the rating)."
)


class Book(BaseModel):
    title: str = Field(description="The book's title")
    price: float = Field(ge=0, description="The price in pounds sterling, as a number")
    in_stock: int = Field(ge=0, description="The number of copies in stock")
    rating: int = Field(ge=1, le=5, description="The star rating, 1 to 5")


class PageTypeLearner:
    """Makes the first book of a page type teach the Plan alone, so concurrent handlers do not each ask the model."""

    def __init__(self) -> None:
        self._lock = asyncio.Lock()
        self._learned = False
        self.sources: Counter[str] = Counter()

    async def extract(self, page: AsyncPage) -> StepResult[Book]:
        if self._learned:
            return self._count(await page.extract(BOOK_INSTRUCTION, Book, mode="compiled"))
        async with self._lock:
            result = await page.extract(BOOK_INSTRUCTION, Book, mode="compiled")
            self._learned = True
            return self._count(result)

    def _count(self, result: StepResult[Book]) -> StepResult[Book]:
        self.sources[str(result.step.source)] += 1
        return result


@dataclass(frozen=True, kw_only=True)
class Unextracted:
    url: str
    outcome: str
    reason: str


@dataclass(frozen=True, kw_only=True)
class CrawlParts:
    crawl: AsyncCrawl
    home: Label[None]
    book: Label[None]
    books: Dataset[Book]


def build_crawl(learner: PageTypeLearner) -> CrawlParts:
    crawl = AsyncCrawl()
    home = crawl.label("home")
    category = crawl.label("category")
    book = crawl.label("book")
    books = crawl.dataset("books", schema=Book)

    @crawl.handle(home)
    async def on_home(ctx: AsyncCrawlContext[None]) -> None:
        side_list = ctx.page.locator(".side_categories ul ul li a")
        await ctx.enqueue_links(side_list, category, limit=CATEGORY_COUNT)

    @crawl.handle(category)
    async def on_category(ctx: AsyncCrawlContext[None]) -> None:
        await ctx.enqueue_links(ctx.page.locator("article.product_pod h3 a"), book)
        await ctx.enqueue_links(ctx.page.locator("li.next a"), ctx.request.label)

    @crawl.handle(book)
    async def on_book(ctx: AsyncCrawlContext[None]) -> None:
        # `ModelAllowanceReachedError` is left to escape: the request ends `skipped` with the reason `model_allowance`.
        result = await learner.extract(ctx.page)
        await ctx.push(books, result.output)

    return CrawlParts(crawl=crawl, home=home, book=book, books=books)


async def unextracted_books(run: AsyncCrawlRun, book: Label[None]) -> list[Unextracted]:
    """The book requests that ended without a Record, keyed by request so a retried request keeps its last outcome."""
    last: dict[str, Unextracted] = {}
    async for outcome in run.outcomes(kinds=["skipped", "failed", "blocked"], labels=[book]):
        if isinstance(outcome, CrawlRequestSkipped):
            why = outcome.reason if outcome.message is None else f"{outcome.reason}: {outcome.message}"
            last[outcome.request.id] = Unextracted(url=outcome.request.url, outcome="skipped", reason=why)
        elif isinstance(outcome, CrawlRequestFailed):
            error = outcome.error
            last[outcome.request.id] = Unextracted(
                url=outcome.request.url, outcome="failed", reason=f"{error.code}: {error.message}"
            )
        elif isinstance(outcome, CrawlRequestBlocked):
            last[outcome.request.id] = Unextracted(
                url=outcome.request.url, outcome="blocked", reason=str(outcome.block_cause)
            )
    return list(last.values())


def print_report(
    report: CrawlRunReport, extracted: list[Book], left: list[Unextracted], sources: Counter[str]
) -> None:
    print(f"Run {report.id}: {report.lifecycle}, completeness {report.completeness}")
    for reason in report.completeness_reasons:
        print(f"  incomplete because: {reason}")
    totals = report.requests
    print(
        f"Requests: {totals.succeeded} succeeded, {totals.failed} failed, {totals.skipped} skipped, "
        f"{totals.blocked} blocked, {totals.pending} pending"
    )
    print(f"Books extracted: {len(extracted)}")
    for extracted_book in extracted:
        print(
            f"  {extracted_book.title} | {extracted_book.price:.2f} GBP | {extracted_book.in_stock} in stock | "
            f"{extracted_book.rating} stars"
        )
    print(f"Extraction steps by source: {dict(sources)} (plan: no model call, model: learned, repair: drifted)")

    print(f"Books left unextracted: {len(left)}")
    for item in left:
        print(f"  {item.url} [{item.outcome}] {item.reason}")

    model = report.usage.model
    price = "unknown" if model.price is None else f"{model.price.amount} {model.price.currency}"
    print(
        f"Model usage: {model.calls} calls, {model.tokens} tokens "
        f"({model.input_tokens} input, {model.output_tokens} output), cost {price}"
    )
    for per_model in model.by_model:
        each = "unknown" if per_model.price is None else f"{per_model.price.amount} {per_model.price.currency}"
        print(f"  {per_model.model}: {per_model.calls} calls, {per_model.tokens} tokens, {each}")
    for allowance in report.usage.model_allowance:
        reached = "never reached" if allowance.reached_at is None else f"reached at {allowance.reached_at}"
        print(f"  allowance {allowance.kind}: {allowance.value}, remaining {allowance.remaining}, {reached}")


async def main() -> int:
    allowance = Decimal(os.environ.get("CRAWL_MODEL_ALLOWANCE_USD", DEFAULT_ALLOWANCE_USD))
    model = os.environ.get("CRAWL_MODEL", DEFAULT_MODEL)
    settings: CrawlRunSettings = {
        "ai": {"model": model, "allowance": {"price": Money(amount=allowance, currency="USD")}},
        "extract": {"mode": "compiled"},
    }

    learner = PageTypeLearner()
    parts = build_crawl(learner)

    async with AsyncFoxhound.connect() as foxhound:
        async with await foxhound.start_crawl(
            parts.crawl, seeds=[seed(START_URL, parts.home)], settings=settings
        ) as run:
            report = await run.finish()
            left = await unextracted_books(run, parts.book)
            extracted = [record.value async for record in run.records(parts.books)]
    print_report(report, extracted, left, learner.sources)
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
