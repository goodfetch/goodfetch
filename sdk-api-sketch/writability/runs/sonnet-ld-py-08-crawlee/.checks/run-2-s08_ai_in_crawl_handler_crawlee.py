"""Crawl three categories of books.toscrape.com with a model-learned extraction and a spending cap.

The model (through the Vercel AI Gateway) is asked once per page type: the selector extractor learns CSS selectors
for the book detail layout, and later books are extracted from those selectors with no model call. A page that no
longer matches what was learned makes the extractor ask the model again. Once the crawl has spent the budget, the
model is cut off, the crawl finishes with the cached selectors only, and the books that could not be extracted are
reported with the reason.

Needs AI_GATEWAY_API_KEY in the environment. CRAWL_BUDGET_USD overrides the 0.50 USD cap.
"""

from __future__ import annotations

import asyncio
import os
import re
from dataclasses import dataclass, field
from decimal import Decimal
from urllib.parse import urljoin

from pydantic import BaseModel, Field, field_validator
from pydantic_ai.exceptions import UnexpectedModelBehavior
from pydantic_ai.messages import ModelMessage, ModelResponse
from pydantic_ai.models import Model, ModelRequestParameters
from pydantic_ai.models.openai import OpenAIChatModel
from pydantic_ai.models.wrapper import WrapperModel
from pydantic_ai.providers.vercel import VercelProvider
from pydantic_ai.settings import ModelSettings

from crawlee import ConcurrencySettings, Request
from crawlee.crawlers import (
    BasicCrawlingContext,
    PydanticAiCrawler,
    PydanticAiCrawlingContext,
    PydanticAiDirectExtractor,
    PydanticAiSelectorExtractor,
)

START_URL = 'https://books.toscrape.com/'
CATEGORY_COUNT = 3
MODEL_NAME = 'anthropic/claude-haiku-4.5'
BUDGET_USD = Decimal(os.environ.get('CRAWL_BUDGET_USD', '0.50'))

# Used only when the price table has no entry for the gateway model (USD per million tokens).
FALLBACK_INPUT_USD_PER_MTOK = Decimal('1.00')
FALLBACK_OUTPUT_USD_PER_MTOK = Decimal('5.00')

RATING_WORDS = {'zero': 0, 'one': 1, 'two': 2, 'three': 3, 'four': 4, 'five': 5}


class Book(BaseModel):
    """One book, as shown on its detail page."""

    title: str = Field(description='The book title, as in the page heading.')
    price: str = Field(description='The price with its currency sign, for example "£51.77".')
    in_stock: int = Field(
        description=(
            'The number of copies in stock. The page shows availability text such as "In stock (22 available)"; '
            'select that text, the number is read from it.'
        )
    )
    star_rating: int = Field(
        description=(
            'The star rating from 0 to 5. The page encodes it as a word in the class attribute of the star-rating '
            'element, for example class="star-rating Three"; select that class attribute, the word is converted.'
        )
    )

    @field_validator('in_stock', mode='before')
    @classmethod
    def _stock_from_text(cls, value: object) -> object:
        if isinstance(value, str):
            match = re.search(r'\d+', value)
            if match is None:
                raise ValueError(f'no stock number in {value!r}')
            return int(match.group())
        return value

    @field_validator('star_rating', mode='before')
    @classmethod
    def _rating_from_class(cls, value: object) -> object:
        if isinstance(value, str):
            for word in reversed(value.lower().split()):
                if word in RATING_WORDS:
                    return RATING_WORDS[word]
            raise ValueError(f'no rating word in {value!r}')
        return value


class BudgetExhausted(Exception):
    """Raised instead of a model call once the crawl has spent its budget."""


@dataclass
class Ledger:
    """What the crawl has spent on the model."""

    budget: Decimal
    spent: Decimal = Decimal(0)
    requests: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    blocked_requests: int = 0
    priced_by_fallback: int = 0

    @property
    def exhausted(self) -> bool:
        return self.spent >= self.budget

    def record(self, response: ModelResponse) -> None:
        usage = response.usage
        self.requests += 1
        self.input_tokens += usage.input_tokens or 0
        self.output_tokens += usage.output_tokens or 0
        try:
            self.spent += response.cost().total_price
        except Exception:  # noqa: BLE001 - any pricing lookup failure falls back to the flat rates
            self.priced_by_fallback += 1
            self.spent += (
                Decimal(usage.input_tokens or 0) * FALLBACK_INPUT_USD_PER_MTOK
                + Decimal(usage.output_tokens or 0) * FALLBACK_OUTPUT_USD_PER_MTOK
            ) / Decimal(1_000_000)


class BudgetedModel(WrapperModel):
    """A model that refuses every request once the ledger says the budget is spent."""

    def __init__(self, wrapped: Model, ledger: Ledger) -> None:
        super().__init__(wrapped)
        self._ledger = ledger

    async def request(
        self,
        messages: list[ModelMessage],
        model_settings: ModelSettings | None,
        model_request_parameters: ModelRequestParameters,
    ) -> ModelResponse:
        if self._ledger.exhausted:
            self._ledger.blocked_requests += 1
            raise BudgetExhausted(
                f'model budget of {self._ledger.budget} USD spent ({self._ledger.spent:.4f} USD); model use is stopped'
            )
        response = await super().request(messages, model_settings, model_request_parameters)
        self._ledger.record(response)
        return response


@dataclass
class Outcome:
    extracted: list[dict[str, object]] = field(default_factory=lambda: [])
    unextracted: dict[str, str] = field(default_factory=lambda: {})
    books_seen: set[str] = field(default_factory=lambda: set())


def build_crawler(ledger: Ledger, outcome: Outcome) -> PydanticAiCrawler:
    model = BudgetedModel(OpenAIChatModel(MODEL_NAME, provider=VercelProvider()), ledger)
    extractor = PydanticAiSelectorExtractor(
        model,
        # A fresh run learns again: no selectors carried over from an earlier run.
        persistence=False,
        # When selector learning fails for a page type, read the page directly (still through the budgeted model).
        fallback=PydanticAiDirectExtractor(model),
    )
    crawler = PydanticAiCrawler(
        extractor=extractor,
        concurrency_settings=ConcurrencySettings(max_concurrency=5),
        max_request_retries=2,
    )

    @crawler.router.default_handler
    async def start_page(context: PydanticAiCrawlingContext) -> None:
        hrefs = context.selector.css('div.side_categories ul.nav-list > li > ul > li > a::attr(href)').getall()
        urls = [urljoin(context.request.url, href) for href in hrefs[:CATEGORY_COUNT]]
        context.log.info(f'Crawling categories: {urls}')
        await context.add_requests([Request.from_url(url, label='category') for url in urls])

    @crawler.router.handler('category')
    async def category_page(context: PydanticAiCrawlingContext) -> None:
        await context.enqueue_links(selector='article.product_pod h3 a', label='book')
        await context.enqueue_links(selector='li.next a', label='category')

    @crawler.router.handler('book')
    async def book_page(context: PydanticAiCrawlingContext) -> None:
        url = context.request.url
        outcome.books_seen.add(url)
        try:
            # The extractor buckets learned selectors by request label, so every book page shares one bucket.
            book = await context.extract(Book)
        except BudgetExhausted as error:
            outcome.unextracted[url] = f'budget: {error}'
            return
        except (UnexpectedModelBehavior, ValueError) as error:
            outcome.unextracted[url] = f'extraction failed: {error}'
            return
        record: dict[str, object] = {'url': url, **book.model_dump()}
        outcome.extracted.append(record)
        await context.push_data(record)

    @crawler.failed_request_handler
    async def failed(context: BasicCrawlingContext, error: Exception) -> None:
        url = context.request.url
        if context.request.label == 'book':
            outcome.unextracted[url] = f'request failed after retries: {error!r}'

    return crawler


def print_report(ledger: Ledger, outcome: Outcome, crawler: PydanticAiCrawler) -> None:
    usage = crawler.ai_usage
    print()
    print(f'Books seen: {len(outcome.books_seen)}')
    print(f'Books extracted: {len(outcome.extracted)}')
    print(f'Books left unextracted: {len(outcome.unextracted)}')
    for url, reason in sorted(outcome.unextracted.items()):
        print(f'  {url}: {reason}')
    print()
    print(f'Model calls: {ledger.requests} (refused after the budget was spent: {ledger.blocked_requests})')
    print(f'Tokens: {usage.input_tokens} input + {usage.output_tokens} output = {usage.total_tokens} total')
    print(f'Cost: {ledger.spent:.4f} USD of a {ledger.budget} USD budget (model {MODEL_NAME})')
    if ledger.priced_by_fallback:
        print(f'  {ledger.priced_by_fallback} call(s) priced with flat fallback rates')
    if ledger.exhausted:
        print('The budget was spent: the rest of the crawl ran on learned selectors only.')


async def main() -> None:
    ledger = Ledger(budget=BUDGET_USD)
    outcome = Outcome()
    crawler = build_crawler(ledger, outcome)
    await crawler.run([START_URL])
    print_report(ledger, outcome, crawler)


if __name__ == '__main__':
    asyncio.run(main())
