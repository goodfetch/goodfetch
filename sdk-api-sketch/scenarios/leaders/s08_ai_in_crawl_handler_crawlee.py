"""Golden scenario 08 on Crawlee for Python 1.11.0: `PydanticAiCrawler` with `PydanticAiSelectorExtractor`.

The extractor asks the model for CSS selectors once per page kind (the request's label) and replays them with no
model call. It copies text, so the schema parses each field's text itself. The library caps no spend: a model wrapper
of the program's own counts each response's price and refuses requests past the budget.
"""

import asyncio
import re
from decimal import Decimal
from typing import Annotated

from pydantic import BaseModel, BeforeValidator, Field
from pydantic_ai.messages import ModelMessage, ModelResponse
from pydantic_ai.models import KnownModelName, ModelRequestParameters
from pydantic_ai.models.wrapper import WrapperModel
from pydantic_ai.settings import ModelSettings

from crawlee.crawlers import (
    BasicCrawlingContext,
    PydanticAiCrawler,
    PydanticAiCrawlingContext,
    PydanticAiDirectExtractor,
    PydanticAiSelectorExtractor,
)

BUDGET = Decimal("0.50")  # USD
RATINGS = {"One": 1, "Two": 2, "Three": 3, "Four": 4, "Five": 5}


def parse_price(text: str) -> str:
    return text.strip().lstrip("£")  # Pydantic parses the Decimal


def parse_stock(text: str) -> int:
    match = re.search(r"(\d+) available", text)
    if match is None:
        raise ValueError(f"no stock count in {text!r}")
    return int(match.group(1))


def parse_rating(css_class: str) -> int:
    return next((RATINGS[word] for word in css_class.split() if word in RATINGS), 0)


class Book(BaseModel):
    title: str = Field(min_length=1)
    price: Annotated[Decimal, BeforeValidator(parse_price)] = Field(
        description="the price text, such as '£51.77'"
    )
    in_stock: Annotated[int, BeforeValidator(parse_stock)] = Field(
        description="the availability text, such as 'In stock (22 available)'"
    )
    rating: Annotated[int, BeforeValidator(parse_rating)] = Field(
        ge=1,
        le=5,
        description="the class of the star rating, such as 'star-rating Three'",
    )


class BudgetReachedError(Exception):
    pass


class BudgetedModel(WrapperModel):
    """Prices every response with genai-prices and refuses each request once the budget is spent."""

    def __init__(self, wrapped: KnownModelName, budget: Decimal) -> None:
        super().__init__(wrapped)
        self.budget = budget
        self.spent = Decimal(0)

    async def request(
        self,
        messages: list[ModelMessage],
        model_settings: ModelSettings | None,
        model_request_parameters: ModelRequestParameters,
    ) -> ModelResponse:
        if self.spent >= self.budget:
            raise BudgetReachedError(f"{self.spent} of {self.budget} USD spent")
        response = await super().request(
            messages, model_settings, model_request_parameters
        )
        self.spent += response.cost().total_price
        return response


model = BudgetedModel("openai:gpt-5-mini", BUDGET)
# The documented setup: a selector generation that fails falls back to reading the page with the model.
extractor = PydanticAiSelectorExtractor(
    model, fallback=PydanticAiDirectExtractor(model)
)
crawler = PydanticAiCrawler(extractor=extractor)
unextracted: list[tuple[str, str]] = []


@crawler.router.default_handler
async def home(context: PydanticAiCrawlingContext) -> None:
    await context.enqueue_links(
        selector=".side_categories ul ul a", label="category", limit=3
    )


@crawler.router.handler("category")
async def category(context: PydanticAiCrawlingContext) -> None:
    await context.enqueue_links(selector="article.product_pod h3 a", label="book")
    await context.enqueue_links(selector="li.next a", label="category")


@crawler.router.handler("book")
async def book(context: PydanticAiCrawlingContext) -> None:
    try:
        # Selectors cached under the label "book": a hit makes no model call, a miss asks the model.
        fields = await context.extract(Book)
    except BudgetReachedError as error:
        unextracted.append((context.request.url, f"budget reached: {error}"))
        return
    await context.push_data(
        {**fields.model_dump(mode="json"), "url": context.request.url}
    )


@crawler.failed_request_handler
async def failed(context: BasicCrawlingContext, error: Exception) -> None:
    unextracted.append((context.request.url, f"{type(error).__name__}: {error}"))


async def main() -> None:
    await crawler.run(["https://books.toscrape.com/"])
    for url, why in unextracted:
        print("unextracted", url, why)
    usage = crawler.ai_usage
    print(
        f"model use: {usage.total_tokens} tokens in {usage.requests} requests, {model.spent} USD"
    )


if __name__ == "__main__":
    asyncio.run(main())
