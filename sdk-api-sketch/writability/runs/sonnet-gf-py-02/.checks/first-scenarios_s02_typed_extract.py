"""Extract one book from books.toscrape.com as a typed object, read by a language model.

The model is chosen by name on the command line or in BOOK_MODEL, as a Goodfetch Model reference:
`anthropic/claude-sonnet-4-5` for a hosted model, `ollama/qwen3:8b` for one served locally.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
from collections.abc import Sequence
from decimal import Decimal
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

from goodfetch_collie import (
    AsyncCollie,
    ExtractionIncompleteError,
    ExtractionInvalidError,
    GoodfetchError,
    ModelUsage,
    Money,
)

URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"

INSTRUCTION = (
    "Extract the book shown on this product page. "
    "title: the book's title. "
    "price: the price in pounds as a decimal number, without the currency symbol (for example 12.34). "
    "in_stock: the number of copies in stock as an integer, taken from the availability line "
    "(for example 'In stock (19 available)' gives 19). "
    "rating: the star rating as an integer from 1 to 5, read from the star-rating element "
    "(its class names the rating in words: One, Two, Three, Four, Five). "
    "description: the product description paragraph, as text. "
    "If a field is not on the page, leave it out; never guess."
)


class Book(BaseModel):
    """The book on the page."""

    model_config = ConfigDict(extra="forbid")

    title: str = Field(min_length=1, description="The book's title.")
    price: Decimal = Field(gt=0, description="The price in pounds, as a decimal number.")
    in_stock: int = Field(ge=0, description="The number of copies in stock.")
    rating: Literal[1, 2, 3, 4, 5] = Field(description="The star rating, from 1 to 5.")
    description: str = Field(min_length=1, description="The product description.")


def field_name(path: Sequence[str | int]) -> str:
    return ".".join(str(part) for part in path) or "(whole object)"


def format_money(money: Money | None) -> str:
    if money is None:
        return "unpriced (the model has no price in the price table)"
    return f"{money.amount} {money.currency}"


def print_usage(usage: ModelUsage) -> None:
    print(f"Model calls:   {usage.calls}")
    print(f"Tokens:        {usage.tokens} (input {usage.input_tokens}, output {usage.output_tokens})")
    print(f"Cost:          {format_money(usage.price)}")


def report_failure(error: GoodfetchError) -> None:
    if isinstance(error, ExtractionIncompleteError):
        missing = error.details.missing
        for item in missing:
            print(f"Field not found: {field_name(item.path)} ({item.reason})", file=sys.stderr)
        print(f"Found so far: {error.details.found}", file=sys.stderr)
    elif isinstance(error, ExtractionInvalidError):
        for issue in error.details.issues:
            print(f"Field invalid: {field_name(issue.path)}: {issue.message}", file=sys.stderr)
    else:
        print(f"{error.code}: {error.message}", file=sys.stderr)
        if error.fix:
            print(f"Fix: {error.fix}", file=sys.stderr)
    if error.usage is not None:
        print_usage(error.usage)


async def run(model: str) -> int:
    async with await AsyncCollie.connect(settings={"ai": {"model": model}}) as collie:
        async with await collie.start(settings={"ai": {"model": model}}) as run_:
            page = await run_.new_page()
            await page.goto(URL)
            result = await page.extract(INSTRUCTION, Book, model=model)
            await run_.finish()
    book = result.output
    print(f"Title:       {book.title}")
    print(f"Price:       £{book.price}")
    print(f"In stock:    {book.in_stock}")
    print(f"Rating:      {book.rating} of 5")
    print(f"Description: {book.description}")
    print_usage(result.usage)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Extract a typed book from books.toscrape.com with a language model.")
    parser.add_argument(
        "--model",
        default=os.environ.get("BOOK_MODEL"),
        help='Model reference, "<provider>/<model id>", such as "ollama/qwen3:8b" (default: $BOOK_MODEL).',
    )
    args = parser.parse_args()
    model: str | None = args.model
    if not model:
        parser.error("choose a model with --model or BOOK_MODEL")
    try:
        return asyncio.run(run(model))
    except GoodfetchError as error:
        report_failure(error)
        return 1


if __name__ == "__main__":
    sys.exit(main())
