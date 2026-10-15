"""Snippet for Models, usage and cost in the SDK API (#475, ADR 0101), on golden scenarios 02, 08 and 11."""

import logging
from datetime import timedelta
from decimal import Decimal
from typing import Literal

from pydantic import BaseModel, Field

from goodfetch import (
    AsyncCrawlRun,
    AsyncGoodfetch,
    AsyncPage,
    ExtractionIncompleteError,
    ExtractionInvalidError,
    GoodfetchError,
    Limits,
    ModelRequest,
    ModelResponse,
    ModelUnavailableError,
    Money,
    ReportedTokens,
    TokenCounts,
)

log = logging.getLogger("books")


class Book(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)  # pounds, exact
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]
    description: str


class Titles(BaseModel):
    titles: list[str]


def usage_fields(usage: TokenCounts) -> dict[str, object]:
    price = "unpriced" if usage.price is None else f"{usage.price.amount} {usage.price.currency}"
    return {"calls": usage.calls, "tokens": usage.tokens, "cached_input": usage.cache_read_tokens, "price": price}


async def typed_book(local: bool) -> Book | None:
    """Golden scenario 02, "Extract a typed object with a chosen model". The model is a Model reference: a provider's
    name, a slash, and that provider's own model ID. The connection's setting serves every call; one call may name its
    own, here a local model served by Ollama, priced zero at its default address."""
    async with (
        AsyncGoodfetch.connect(settings={"ai": {"model": "anthropic/claude-sonnet-4-5"}}) as gf,  # env:ANTHROPIC_API_KEY
        await gf.collie.start() as run,
    ):
        page = await run.new_page()
        await page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html")
        try:
            if local:
                result = await page.extract("the book on this page", Book, model="ollama/qwen3:8b")
            else:
                result = await page.extract("the book on this page", Book)
        except ExtractionIncompleteError as error:
            log.warning("fields not on the page: %s", error.details.missing)
            return None
        except ExtractionInvalidError as error:
            # Names the model and the strategy; what the failed attempts cost is on the error too.
            spent = usage_fields(error.usage) if error.usage is not None else {}
            log.error("%s never fitted the schema with %s: %s", error.details.model, error.details.strategy, spent)
            raise
        log.info("extracted: %s", usage_fields(result.usage))  # always there, priced from the price table
        return result.output


async def in_house_model(request: ModelRequest) -> ModelResponse:
    """The program's own Model provider (ADR 0033's SDK-handler form): Goodfetch's own types, no provider SDK."""
    text = "{}"  # calls the program's own inference service
    return ModelResponse(text=text, finish_reason="stop", usage=ReportedTokens(input_tokens=812, output_tokens=64))


async def providers() -> None:
    """A gateway, a declared OpenAI-compatible server, and the program's own handler. Capabilities are declared, never
    guessed from the name; prices are set only in configuration."""
    async with AsyncGoodfetch.connect(
        model_providers={"inhouse": in_house_model},  # "inhouse/<anything>" reaches it
        settings={
            "ai": {
                "model": "vercel/openai/gpt-5-mini",  # the Vercel AI Gateway, env:AI_GATEWAY_API_KEY (ADR 0076)
                "structured_output": "auto",
                "providers": {
                    "lab": {
                        "format": "openai",
                        "base_url": "http://gpu-box.internal:8000/v1",
                        "api_key": "env:LAB_VLLM_KEY",  # a Secret reference, sent only to base_url
                        "models": {
                            "qwen3-vl-32b": {"image_input": True, "context_window": 131_072, "strategies": ["native", "prompt"]}
                        },
                    }
                },
            },
            "timeout": {"model": timedelta(minutes=2)},  # a slow server: one model request may take 2 minutes
        },
    ) as gf:
        async with await gf.collie.start(settings={"ai": {"model": "lab/qwen3-vl-32b"}}) as run:
            page = await run.new_page()
            await page.goto("https://books.toscrape.com/")
            await page.act("open the Travel category")
            # The program's own model for one call: it gets the prompt strategy, and only configuration prices it.
            await page.extract("the books listed", Titles, model="inhouse/ranker-v2")


async def capped_run(gf: AsyncGoodfetch) -> None:
    """Run totals, what remains of each Limit, and the uncapped mark (ADR 0041). The price Limit and `usage.price` are
    the same `Money`."""
    limits = Limits(price=Money(amount=Decimal("0.20"), currency="USD"), model_calls=40)
    async with await gf.collie.start(limits=limits) as run:
        page = await run.new_page()
        await page.goto("https://books.toscrape.com/")
        await page.act("open the Travel category")
        usage = await run.usage()
        log.info("so far: %s, browser %s, uncapped %s", usage_fields(usage.model), usage.browser_time, usage.uncapped)
        for limit in usage.limits:
            log.info("%s %s: %s remain (set by %s)", limit.scope, limit.kind, limit.remaining, limit.set_by)


async def with_outage(page: AsyncPage) -> None:
    """A provider outage is retryable and says when to come back; a rejected key or an exhausted credit is not."""
    try:
        await page.act("add the first book to the basket")
    except ModelUnavailableError as error:
        if error.details.reason == "rate_limited":
            log.warning("rate limited by %s, retry after %s", error.details.model, error.details.retry_after)
            return
        raise
    except GoodfetchError as error:
        if error.code in ("model_unauthorized", "model_funds_exhausted"):
            log.error("%s", error.message)
        raise


async def report_spend(run: AsyncCrawlRun) -> None:
    """Golden scenario 08's Usage report. Its "stop model use, finish without the model" is "Capping a crawl's model use
    without stopping its Run" (#513): here the price Limit stops the crawl."""
    usage = await run.usage()
    for used in usage.model.by_model:
        log.info("%s: %s", used.model, usage_fields(used))
    log.info("spent %s, uncapped %s", usage.price, usage.uncapped)
