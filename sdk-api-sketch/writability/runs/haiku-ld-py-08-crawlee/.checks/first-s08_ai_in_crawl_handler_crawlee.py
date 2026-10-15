"""Crawl the first three categories of books.toscrape.com and extract every book with a language model.

Navigation uses plain CSS links (category side list, product links, "next" pages). Book facts are extracted by a
`PydanticAiSelectorExtractor`: the model is asked once for the book page layout, the learned CSS selectors then
extract later books without a model call, and the model is asked again only when a page no longer matches them.

All model use goes through `BudgetedModel`, which counts the cost of every model response and refuses further
requests once `BUDGET_USD` is spent. Learned selectors keep working after that, at no cost, and books whose layout
no longer matches are reported as unextracted.
"""

from __future__ import annotations

import asyncio
import re
from dataclasses import dataclass, field
from decimal import Decimal

from pydantic import BaseModel, Field
from pydantic_ai.messages import ModelMessage, ModelResponse
from pydantic_ai.models import Model, ModelRequestParameters, infer_model
from pydantic_ai.models.wrapper import WrapperModel
from pydantic_ai.settings import ModelSettings

from crawlee.crawlers import PydanticAiCrawler, PydanticAiCrawlingContext, PydanticAiSelectorExtractor

START_URL = 'https://books.toscrape.com/'
MODEL_NAME = 'openai:gpt-5.4-nano'
BUDGET_USD = Decimal('0.50')
CATEGORY_LIMIT = 3

CATEGORY = 'CATEGORY'
BOOK = 'BOOK'

STAR_WORDS = {'one': 1, 'two': 2, 'three': 3, 'four': 4, 'five': 5}


class BookDetails(BaseModel):
    """What the model extracts from one book page. The fields are read as text and normalised in Python."""

    title: str = Field(description='The book title, from the page heading')
    price: str = Field(description='The price as displayed, including its currency symbol')
    availability: list[str] = Field(
        description='The text pieces of the stock line, for example "In stock (22 available)"'
    )
    star_rating: str = Field(description='The star-rating element with its class, for example "star-rating Three"')


class BookRecord(BaseModel):
    """One extracted book, as pushed to the dataset."""

    url: str
    title: str
    price: str
    number_in_stock: int
    star_rating: int


class BudgetExhaustedError(Exception):
    """Raised in place of a model request once the spend has reached the budget."""


@dataclass
class SpendLedger:
    """Model spend of the whole crawl, counted per model response."""

    budget_usd: Decimal
    requests: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    cost_usd: Decimal = Decimal(0)
    unpriced_requests: int = 0

    @property
    def exhausted(self) -> bool:
        """Whether no further model request may be made.

        A response whose price is unknown also stops model use, because the spend can no longer be verified.
        """
        return self.cost_usd >= self.budget_usd or self.unpriced_requests > 0

    def record(self, response: ModelResponse) -> None:
        """Add the tokens and cost of one model response."""
        self.requests += 1
        self.input_tokens += response.usage.input_tokens
        self.output_tokens += response.usage.output_tokens
        cost = _response_cost(response)
        if cost is None:
            self.unpriced_requests += 1
        else:
            self.cost_usd += cost


def _response_cost(response: ModelResponse) -> Decimal | None:
    """Return the USD cost of a response, or `None` when no price is known for it."""
    if response.usage.cost is not None:
        return response.usage.cost
    if response.model_name is None:
        return None
    try:
        return response.cost().total_price
    except (LookupError, ValueError):
        return None


class BudgetedModel(WrapperModel):
    """Model wrapper that makes no provider request once the ledger is exhausted."""

    def __init__(self, wrapped: Model, ledger: SpendLedger) -> None:
        super().__init__(wrapped)
        self._ledger = ledger

    async def request(
        self,
        messages: list[ModelMessage],
        model_settings: ModelSettings | None,
        model_request_parameters: ModelRequestParameters,
    ) -> ModelResponse:
        if self._ledger.exhausted:
            raise BudgetExhaustedError(
                f'the model budget of {self._ledger.budget_usd} USD is spent '
                f'({self._ledger.cost_usd} USD counted, {self._ledger.unpriced_requests} unpriced responses)'
            )
        response = await super().request(messages, model_settings, model_request_parameters)
        self._ledger.record(response)
        return response


def _stock_count(pieces: list[str]) -> int | None:
    """Read the number of copies in stock from the stock line, or return `None` when it is not there."""
    text = ' '.join(piece.strip() for piece in pieces if piece.strip())
    match = re.search(r'\((\d+) available\)', text)
    return int(match.group(1)) if match else None


def _star_count(text: str) -> int | None:
    """Read the star rating word, such as "Three", from the star-rating text."""
    match = re.search(r'\b(one|two|three|four|five)\b', text, re.IGNORECASE)
    return STAR_WORDS[match.group(1).lower()] if match else None


@dataclass
class Unextracted:
    """A book that the crawl reached but did not extract, with the reason."""

    url: str
    reason: str


@dataclass
class CrawlState:
    """Results collected while the crawl runs."""

    books: list[BookRecord] = field(default_factory=list)
    learned_only: int = 0
    unextracted: list[Unextracted] = field(default_factory=list)


async def main() -> None:
    ledger = SpendLedger(budget_usd=BUDGET_USD)
    extractor = PydanticAiSelectorExtractor(
        BudgetedModel(infer_model(MODEL_NAME), ledger),
        persistence=False,
    )
    crawler = PydanticAiCrawler(extractor=extractor)
    state = CrawlState()

    @crawler.router.default_handler
    async def home(context: PydanticAiCrawlingContext) -> None:
        # Navigation only: the first categories of the side list.
        await context.enqueue_links(selector='div.side_categories ul ul a', label=CATEGORY, limit=CATEGORY_LIMIT)

    @crawler.router.handler(CATEGORY)
    async def category(context: PydanticAiCrawlingContext) -> None:
        # Navigation only: every product link on the listing page, then the next listing page.
        await context.enqueue_links(selector='article.product_pod h3 a', label=BOOK)
        await context.enqueue_links(selector='li.next a', label=CATEGORY)

    @crawler.router.handler(BOOK)
    async def book(context: PydanticAiCrawlingContext) -> None:
        url = context.request.url
        requests_before = ledger.requests
        try:
            details = await context.extract(BookDetails, cache_tag=BOOK)
        except Exception as error:
            if isinstance(error, BudgetExhaustedError):
                reason = f'the page did not match the learned selectors and {error}'
            else:
                reason = f'extraction failed: {type(error).__name__}: {error}'
            state.unextracted.append(Unextracted(url, reason))
            return

        stock = _stock_count(details.availability)
        if stock is None:
            state.unextracted.append(Unextracted(url, f'no stock count in {details.availability!r}'))
            return
        stars = _star_count(details.star_rating)
        if stars is None:
            state.unextracted.append(Unextracted(url, f'no star rating in {details.star_rating!r}'))
            return

        record = BookRecord(
            url=url,
            title=details.title,
            price=details.price,
            number_in_stock=stock,
            star_rating=stars,
        )
        state.books.append(record)
        if ledger.requests == requests_before:
            state.learned_only += 1
        await context.push_data(record.model_dump())

    stats = await crawler.run([START_URL])

    print(f'Books extracted: {len(state.books)} ({state.learned_only} with no model call)')
    print(f'Books unextracted: {len(state.unextracted)}')
    for item in state.unextracted:
        print(f'  {item.url}: {item.reason}')
    print(f'Failed requests: {stats.requests_failed}')
    total_tokens = ledger.input_tokens + ledger.output_tokens
    print(f'Model requests: {ledger.requests}')
    print(f'Tokens: {ledger.input_tokens} input + {ledger.output_tokens} output = {total_tokens}')
    print(f'Cost: {ledger.cost_usd:.6f} USD of a {BUDGET_USD:.2f} USD budget')
    if ledger.unpriced_requests:
        print(f'Responses without a known price: {ledger.unpriced_requests}')


if __name__ == '__main__':
    asyncio.run(main())
