"""Scenario 11: one instruction, reached by an Agent on books.toscrape.com.

The Agent gets the instruction and nothing else; the program states the Success check, the step cap and the spend cap.
The steps the Agent took are kept as Python page calls and in a Plan file, so the same steps can be repeated without a
model. Only the chosen model's provider receives the task's content: navigation stays on the site's own host.
"""

import re
from collections.abc import Sequence
from datetime import timedelta
from decimal import Decimal
from typing import Literal, TypeAlias

from goodfetch_collie import Collie
from goodfetch_collie_client import (
    AgentResult,
    AgentStep,
    Limits,
    Locator,
    Money,
    Page,
    RunSettings,
    Stopped,
    expect,
)
from pydantic import BaseModel, Field

SITE = "books.toscrape.com"
START_URL = f"https://{SITE}/"
TASK = "Find the cheapest book rated five stars in the Travel category and give its title and price."
MODEL = "vercel/anthropic/claude-sonnet-4.5"
MAX_STEPS = 25
SPEND_CAP = Money(amount=Decimal("0.20"), currency="USD")
RUN_TIMEOUT = timedelta(minutes=15)
PLAN_FILE = "scenarios/s11_agent_goal.plan.json"
BOOK_PAGE = re.compile(r"^https://books\.toscrape\.com/catalogue/[^/]+_\d+/index\.html$")

RUN_SETTINGS: RunSettings = {
    "ai": {"model": MODEL, "allowance": {"price": SPEND_CAP}},
    "agent": {"max_steps": MAX_STEPS, "domains": [SITE], "human_pause": False},
}

Ending: TypeAlias = Literal["verified", "reported_done", "stopped_by_limit", "unresolved"]


class BookAnswer(BaseModel):
    """The typed answer the Agent must give."""

    title: str = Field(description="The title of the book, as its page shows it.")
    price: str = Field(description="The price as its page shows it, with the currency symbol.")


class StepRecord(BaseModel):
    """One Agent step: its page calls, repeatable without a model."""

    index: int
    origin: str
    url: str
    actions: list[str]


class FindResult(BaseModel):
    """The answer, and how the work ended."""

    title: str | None
    price: str | None
    ending: Ending
    stop_reason: str | None
    steps: list[StepRecord]


def check_book_page(page: Page) -> None:
    """The program's Success check: a book page in the Travel category with a five-star rating."""
    expect(page).to_have_url(BOOK_PAGE)
    expect(page.locator("ul.breadcrumb li").nth(2)).to_have_text("Travel")
    expect(page.locator("p.star-rating.Five")).to_have_count(1)


def ending_of(result: AgentResult[BookAnswer]) -> tuple[Ending, str | None]:
    """Verified by the Success check, stopped by a Limit or step cap, reported as done by the model, or unresolved."""
    if result.verdict == "verified":
        return "verified", None
    ended = result.ended
    if isinstance(ended, Stopped):
        return "stopped_by_limit", ended.reason
    reported = result.reported
    if reported is not None and reported.claim == "done":
        return "reported_done", None
    return "unresolved", None


def step_records(steps: Sequence[AgentStep[Locator]]) -> list[StepRecord]:
    """Each step's actions as the Python page calls the engine rendered for them."""
    return [
        StepRecord(
            index=step.index,
            origin=step.origin,
            url=step.url,
            actions=[action.code if action.code is not None else action.operation for action in step.actions],
        )
        for step in steps
    ]


def find_cheapest_five_star_travel_book() -> FindResult:
    """Runs the Agent once on the site and returns the title, the price and how the work ended."""
    with Collie.connect(project=None) as collie:
        with collie.start(
            plan=PLAN_FILE,
            settings=RUN_SETTINGS,
            timeout=RUN_TIMEOUT,
            limits=Limits(price=SPEND_CAP),
        ) as run:
            page = run.new_page()
            page.goto(START_URL)
            result = page.agent(TASK, schema=BookAnswer, success_check=check_book_page).result()
            run.finish()
            ending, stop_reason = ending_of(result)
            answer = result.output
            return FindResult(
                title=answer.title if answer is not None else None,
                price=answer.price if answer is not None else None,
                ending=ending,
                stop_reason=stop_reason,
                steps=step_records(result.steps),
            )


def main() -> None:
    print(find_cheapest_five_star_travel_book().model_dump_json(indent=2))


if __name__ == "__main__":
    main()
