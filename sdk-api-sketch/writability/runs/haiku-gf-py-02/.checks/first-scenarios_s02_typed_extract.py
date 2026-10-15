"""Extract one book page as a typed object with a language model (Collie's AI verbs, ADR 0091).

The model is named by a Model reference, `provider/model-id`, given with `--model`. A locally served model, such as
`ollama/qwen3:8b`, goes through the same call as a hosted one. Every value is checked against the Pydantic model, and a
field that is missing or of the wrong type is reported by name.
"""

import argparse
import sys
from collections.abc import Sequence
from typing import Literal

from pydantic import BaseModel, Field

from goodfetch_collie import Collie
from goodfetch_collie_client import (
    ExtractionIncompleteError,
    ExtractionInvalidError,
    GoodfetchError,
    ModelReference,
    ModelUsage,
    StepResult,
)

BOOK_URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"
DEFAULT_MODEL: ModelReference = "vercel/openai/gpt-4o-mini"
INSTRUCTION = (
    "Read this book's product page and return its title, its price in pounds, the number of copies in stock, "
    "its star rating from one to five, and its description."
)


class Book(BaseModel):
    title: str = Field(description="The book's title, as text.")
    price_pounds: float = Field(
        ge=0,
        description="The price in pounds as a plain decimal number, for example 51.77, without the currency sign.",
    )
    copies_in_stock: int = Field(
        ge=0,
        description="The number of copies in stock, as an integer, from the availability text.",
    )
    star_rating: Literal[1, 2, 3, 4, 5] = Field(
        description="The star rating as an integer from one to five. The page names it in words, such as Three.",
    )
    description: str = Field(description="The book's description, as text.")


def extract_book(model: ModelReference) -> tuple[Book, ModelUsage]:
    with Collie.connect(settings={"ai": {"model": model}}) as collie:
        with collie.start() as run:
            page = run.new_page()
            page.goto(BOOK_URL)
            result: StepResult[Book] = page.extract(INSTRUCTION, Book, mode="model")
            run.finish()
    return result.output, result.usage


def field_name(path: Sequence[str | int]) -> str:
    return ".".join(str(part) for part in path) or "(whole answer)"


def describe_usage(usage: ModelUsage | None) -> str:
    if usage is None:
        return "tokens: none used"
    if usage.price is None:
        cost = "cost: unknown, the price table has no price for this model"
    else:
        cost = f"cost: {usage.price.amount} {usage.price.currency} (list-price estimate)"
    return (
        f"tokens: {usage.tokens} (input {usage.input_tokens}, output {usage.output_tokens}), "
        f"model calls: {usage.calls}, {cost}"
    )


def format_book(book: Book) -> str:
    return "\n".join(
        [
            f"title: {book.title}",
            f"price: {book.price_pounds:.2f} GBP",
            f"copies in stock: {book.copies_in_stock}",
            f"star rating: {book.star_rating}",
            f"description: {book.description}",
        ]
    )


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Extract the book page as a typed object with a language model.")
    parser.add_argument(
        "--model",
        default=DEFAULT_MODEL,
        help="Model reference, provider/model-id, such as ollama/qwen3:8b for a local model.",
    )
    args = parser.parse_args(argv)
    model: ModelReference = str(args.model)

    try:
        book, usage = extract_book(model)
    except ExtractionIncompleteError as error:
        for missing in error.details.missing:
            print(f"field not found: {field_name(missing.path)} ({missing.reason})", file=sys.stderr)
        print(describe_usage(error.usage), file=sys.stderr)
        return 1
    except ExtractionInvalidError as error:
        for issue in error.details.issues:
            print(f"field of the wrong type: {field_name(issue.path)}: {issue.message}", file=sys.stderr)
        print(describe_usage(error.usage), file=sys.stderr)
        return 1
    except GoodfetchError as error:
        print(f"extraction failed ({error.code}): {error.message}", file=sys.stderr)
        print(describe_usage(error.usage), file=sys.stderr)
        return 1

    print(format_book(book))
    print(describe_usage(usage))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
