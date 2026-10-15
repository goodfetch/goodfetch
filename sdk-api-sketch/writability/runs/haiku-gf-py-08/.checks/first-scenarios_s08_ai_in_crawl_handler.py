"""Scenario 08: crawl three book categories, extracting each book's facts with a language model.

The model is asked once per page type. The book pages' extraction is compiled into the Run's Plan by the first book
and replayed for the rest with no model call, until a page no longer matches what was learned (then one repair). The
Run's model allowance caps the model spend; books that need a model after it is reached are skipped with that reason,
and the crawl finishes without the model. Book handlers run one at a time so the first book teaches the layout before
the others start.
"""

from decimal import Decimal

from pydantic import BaseModel, Field

from goodfetch_foxhound import (
    Crawl,
    CrawlContext,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestOutcome,
    CrawlRequestSkipped,
    CrawlRequestSucceeded,
    CrawlRunSettings,
    Foxhound,
    LinkInfo,
    Money,
    seed,
)

HOME_URL = "https://books.toscrape.com/"
CATEGORY_COUNT = 3
MODEL = "vercel/openai/gpt-4o-mini"
BUDGET = Money(amount=Decimal("0.50"), currency="USD")

BOOK_INSTRUCTION = (
    "This is a book's detail page. Give the book's title, its price as shown, the number of copies in stock taken "
    "from the availability text, and its star rating as a whole number from 1 to 5 read from the star-rating class name."
)


class BookFacts(BaseModel):
    title: str
    price: str
    in_stock: int = Field(ge=0)
    star_rating: int = Field(ge=1, le=5)


class BookRecord(BookFacts):
    url: str
    category: str


crawl = Crawl()
home = crawl.label("home")
category = crawl.label("category", payload=str)
book = crawl.label("book", payload=str, settings={"crawl": {"handler": {"concurrency": 1}}})
books = crawl.dataset("books", schema=BookRecord, key=["url"])

RUN_SETTINGS: CrawlRunSettings = {"ai": {"model": MODEL, "allowance": {"price": BUDGET}}}


def category_name(link: LinkInfo) -> str:
    return link.text.strip()


@crawl.handle(home)
def handle_home(ctx: CrawlContext[None]) -> None:
    ctx.enqueue_links(ctx.page.locator("ul.nav-list ul a"), category, category_name, limit=CATEGORY_COUNT)


@crawl.handle(category)
def handle_category(ctx: CrawlContext[str]) -> None:
    page = ctx.page
    ctx.enqueue_links(page.locator("article.product_pod h3 a"), book, ctx.request.payload)
    ctx.enqueue_links(page.locator("li.next a"), category, ctx.request.payload)


@crawl.handle(book)
def handle_book(ctx: CrawlContext[str]) -> None:
    facts = ctx.page.extract(BOOK_INSTRUCTION, BookFacts, mode="compiled").output
    ctx.push(
        books,
        BookRecord(
            title=facts.title,
            price=facts.price,
            in_stock=facts.in_stock,
            star_rating=facts.star_rating,
            url=ctx.request.url,
            category=ctx.request.payload,
        ),
    )


def money(value: Money | None) -> str:
    if value is None:
        return "unpriced"
    return f"{value.amount} {value.currency}"


def describe(outcome: CrawlRequestOutcome) -> str:
    if isinstance(outcome, CrawlRequestSkipped):
        return f"skipped ({outcome.reason}): {outcome.message or 'no message'}"
    if isinstance(outcome, CrawlRequestFailed):
        return f"failed ({outcome.error.code}): {outcome.error.message}"
    if isinstance(outcome, CrawlRequestBlocked):
        return f"blocked ({outcome.block_cause})"
    return "not extracted (unrecognised outcome)"


def main() -> None:
    with Foxhound.connect() as fox:
        run = fox.start_crawl(crawl, seeds=[seed(HOME_URL, home)], settings=RUN_SETTINGS)
        report = run.finish()
        latest: dict[str, CrawlRequestOutcome] = {}
        for outcome in run.outcomes(labels=[book]):
            latest[outcome.request.id] = outcome

    extracted = sum(1 for outcome in latest.values() if isinstance(outcome, CrawlRequestSucceeded))
    left_out = [f"{outcome.request.url}: {describe(outcome)}" for outcome in latest.values() if not isinstance(outcome, CrawlRequestSucceeded)]

    usage = report.usage.model
    print(f"Run {report.lifecycle}, completeness {report.completeness}")
    print(f"Books extracted: {extracted}")
    print(f"Books left unextracted: {len(left_out)}")
    for line in left_out:
        print(f"  {line}")
    print(f"Model calls: {usage.calls}")
    print(f"Tokens: {usage.tokens} (input {usage.input_tokens}, output {usage.output_tokens})")
    print(f"Cost (list price): {money(usage.price)}")
    for allowance in report.usage.model_allowance:
        print(
            f"Allowance {allowance.kind}: value {allowance.value}, remaining {allowance.remaining}, "
            f"reached at {allowance.reached_at}"
        )


if __name__ == "__main__":
    main()
