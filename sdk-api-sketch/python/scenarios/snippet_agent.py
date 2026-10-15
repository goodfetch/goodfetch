"""Snippet for the Agent in Collie's SDK API (#474, ADR 0100), on golden scenario 11."""

import logging
from decimal import Decimal

from pydantic import BaseModel, Field

from goodfetch import (
    AgentToolFailedError,
    AsyncAgentResult,
    AsyncGoodfetch,
    AsyncPage,
    AsyncRun,
    Goodfetch,
    JsonValue,
    Limits,
    Money,
    Page,
    Stopped,
    agent_tool,
    expect,
)

log = logging.getLogger("travel")


class Answer(BaseModel):
    title: str
    price: str = Field(pattern=r"^£\d+\.\d{2}$")


async def travel_book_check(page: AsyncPage) -> None:
    """The Success check: a book page in Travel with five stars. A model's "done" is only a claim."""
    await expect(page.locator(".breadcrumb")).to_contain_text("Travel")
    await expect(page.locator(".product_main .star-rating")).to_have_class("star-rating Five")


async def cheapest_five_star_travel_book() -> Answer | None:
    """Golden scenario 11, "Reach a goal from one instruction, with limits and a verdict". The Run caps the price at
    0.20 USD (a Limit, which stops the Run), the task caps itself at 25 steps (which stops only the task), and the
    program, not the model, states what success looks like. With `plan=`, the steps are kept, and a later run repeats
    them with no model call."""
    async with AsyncGoodfetch.connect() as gf:
        async with await gf.collie.start(
            plan="travel.plan.json", limits=Limits(price=Money(amount=Decimal("0.20"), currency="USD"))
        ) as run:
            page = await run.new_page()
            await page.goto("https://books.toscrape.com/")
            async with page.agent(
                "find the cheapest book rated five stars in the Travel category and give its title and price",
                schema=Answer,
                max_steps=25,
                # By default the task stays on toscrape.com, where it starts; this narrows it to one host.
                domains=["books.toscrape.com"],
                success_check=travel_book_check,
            ) as task:
                # The steps as they happen, from step 1, none dropped.
                async for step in task.steps():
                    log.info("step %d (%s) at %s: %s", step.index, step.origin, step.url, [a.code or a.operation for a in step.actions])
                result = await task.result()
            log.info("agent done: %s tokens, price %s, %s", result.usage.tokens, result.usage.price, result.verdict)
            return report(result)


def report(result: AsyncAgentResult[Answer]) -> Answer | None:
    """How the work ended, its Verdict and the model's claim are three separate fields (ADR 0079)."""
    if isinstance(result.ended, Stopped):
        # `max_steps`, `no_progress` and `timeout` name the setting that stopped it in `source`; `limit` names the Limit.
        claim = result.reported.claim if result.reported else None
        log.warning("agent stopped (%s); partial %s; claimed %s", result.ended.reason, result.partial, claim)
        return None
    if result.verdict != "verified":
        # `rejected`: the Success check failed after the model said done; `needs-review`: nothing proved it.
        log.warning("agent %s: %s", result.verdict, result.reported)
        return None
    return result.require_verified()


class Owned(BaseModel):
    title: str


@agent_tool(description="Whether the library already owns a book with this exact title", read_only=True)
async def already_owned(args: Owned, page: AsyncPage) -> JsonValue:
    """An Agent tool: the program's own function as an action. Read-only, so it is no Effect."""
    return {"owned": args.title in {"Sharp Objects"}}


async def cheapest_unowned_book(run: AsyncRun) -> Answer:
    """A task whose result alone is read, with a tool. `require_verified()` raises `NotVerifiedError`, with every
    reason, when the Verdict is not `verified`."""
    page = await run.new_page()
    await page.goto("https://books.toscrape.com/catalogue/category/books/travel_2/index.html")
    try:
        result = await page.agent(
            "find the cheapest five-star book here that the library does not own yet",
            schema=Answer,
            tools=[already_owned],
            success_check=travel_book_check,
        ).result()
    except AgentToolFailedError as error:
        log.error("tool %s raised: %s", error.details.tool, error.details.exception_message)
        # Whatever ended the task, the steps taken and the Grounded partial answer travel on the error.
        log.warning("partial answer: %s", error.agent_task.partial if error.agent_task else None)
        raise
    # Each page action carries its code in Python, to turn the task into plain page API calls.
    log.debug("as code: %s", [a.code for s in result.steps for a in s.actions if a.code])
    return result.require_verified()


def sync_task(page: Page) -> str | None:
    """The sync API keeps the same shape: a handle with `result()`."""
    result = page.agent("open the Travel category and report how many books it lists").result()
    return result.reported.message if result.reported else None


def sync_connect() -> None:
    with Goodfetch.connect() as gf, gf.collie.start() as run:
        sync_task(run.new_page())
