"""Golden scenario 02, "Extract a typed object with a chosen model" (ADRs 0082, 0091, 0101)."""

import asyncio
import re
import sys
from decimal import Decimal
from typing import Literal

from pydantic import BaseModel, Field

from goodfetch_collie import (
    AsyncCollie,
    ExtractionIncompleteError,
    ExtractionInvalidError,
    expect,
)


class Book(BaseModel):
    title: str
    price: Decimal = Field(ge=0, decimal_places=2)  # pounds, exact
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]
    description: str


# A Model reference names its provider, then that provider's model ID: a model Ollama serves locally is one more
# provider, priced zero at its default address (ADR 0101).
MODEL = "ollama/qwen3:8b" if "--local" in sys.argv else "anthropic/claude-sonnet-4-5"


async def main() -> None:
    # Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic.
    async with AsyncCollie.connect() as collie, await collie.start() as run:
        page = await run.new_page()
        await page.goto(
            "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"
        )
        await expect(page).to_have_title(
            re.compile("A Light in the Attic")
        )  # not another page, nor an error page
        try:
            # Every answer is checked against Book; a provider that rejects native structured output gets a forced
            # tool call, then the schema in the prompt (`ai.structured_output: auto`).
            result = await page.extract("the book on this page", Book, model=MODEL)
        except ExtractionIncompleteError as error:
            print(
                "Not on the page:",
                [field.path for field in error.details.missing],
                file=sys.stderr,
            )
            raise
        except ExtractionInvalidError as error:
            print(
                "Not of the expected type:",
                [(i.path, i.message) for i in error.details.issues],
                file=sys.stderr,
            )
            raise  # its `usage` says what the failed attempts spent
        usage = result.usage
        price = (
            "unpriced"
            if usage.price is None
            else f"{usage.price.amount} {usage.price.currency}"
        )
        print(
            result.output,
            f"{usage.tokens} tokens ({usage.input_tokens} in, {usage.output_tokens} out), {price}",
        )
        await run.finish()


asyncio.run(main())
