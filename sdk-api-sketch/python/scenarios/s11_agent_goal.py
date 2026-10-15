"""Golden scenario 11, "Reach a goal from one instruction, with limits and a verdict" (ADRs 0013, 0079, 0091, 0100, 0101)."""

import asyncio
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from decimal import Decimal
from typing import Literal

from pydantic import BaseModel, Field

from goodfetch_collie import (
    AsyncAgentResult,
    AsyncCollie,
    AsyncPage,
    Limits,
    Money,
    Stopped,
    expect,
)


class Answer(BaseModel):
    title: str
    price: str = Field(pattern=r"^£\d+\.\d{2}$")


@dataclass(frozen=True)
class Outcome:
    """How the work ended, with what it found: the answer once finished, the Grounded partial one when stopped."""

    ended: Literal["verified", "reported", "stopped"]
    answer: Answer | Mapping[str, object] | Sequence[object] | None
    why: str


async def travel_book_check(page: AsyncPage) -> None:
    """The Success check: the model's "done" is only a claim until this holds on the page it reached."""
    await expect(page.locator(".breadcrumb")).to_contain_text("Travel")
    await expect(page.locator(".product_main .star-rating")).to_have_class(
        "star-rating Five"
    )


def outcome(result: AsyncAgentResult[Answer]) -> Outcome:
    if isinstance(result.ended, Stopped):
        return Outcome(
            ended="stopped",
            answer=result.partial,
            why=result.ended.limit or result.ended.reason,
        )
    if result.verdict == "verified":
        return Outcome(
            ended="verified", answer=result.require_verified(), why="success check"
        )
    return Outcome(
        ended="reported",
        answer=result.output,
        why=result.reported.claim if result.reported else "none",
    )


async def main() -> None:
    # No telemetry exists to switch off: only the site and the model's provider are reached (ADR 0043).
    # Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic.
    async with AsyncCollie.connect() as collie:
        async with await collie.start(
            # The task is one Plan entry: once a Run is verified, a later one replays it with no model call.
            plan="travel.plan.json",
            # A Limit: reaching it stops the Run too.
            limits=Limits(price=Money(amount=Decimal("0.20"), currency="USD")),
        ) as run:
            page = await run.new_page()
            await page.goto("https://books.toscrape.com/")
            result = await page.agent(
                "find the cheapest book rated five stars in the Travel category and give its title and price",
                schema=Answer,
                model="anthropic/claude-sonnet-4-5",
                max_steps=25,  # stops only the task
                # Enforced by the engine on every navigation the task makes, whatever the pages say (ADRs 0014, 0100).
                domains=["books.toscrape.com"],
                success_check=travel_book_check,
            ).result()
            await run.finish()
    print(outcome(result))


asyncio.run(main())
