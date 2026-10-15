"""Find the cheapest five-star Travel book on books.toscrape.com with Browser Use 0.13.11.

One instruction goes to a model-driven agent. The program caps the run at 25 steps and
0.20 USD, checks the page the run ends on against a stated condition, and prints the
steps taken so that they can be replayed without a model.
"""

import asyncio
import os
import re
from typing import Literal

# Only the model provider may receive task content: no telemetry, no cloud sync.
os.environ["ANONYMIZED_TELEMETRY"] = "false"
os.environ["BROWSER_USE_CLOUD_SYNC"] = "false"

from pydantic import BaseModel, Field  # noqa: E402

from browser_use import (  # noqa: E402
    Agent,
    AgentHistoryList,
    BrowserProfile,
    BrowserSession,
    ChatOpenAI,
    Tools,
)

START_URL = "https://books.toscrape.com/"
ALLOWED_DOMAIN = "books.toscrape.com"
MAX_STEPS = 25
MAX_COST_USD = 0.20
# Stop before a step could push the spend past the cap; one step costs far less than this.
COST_RESERVE_USD = 0.02
MODEL = "gpt-4o-mini"
# Assumed list prices in USD per million tokens for MODEL: input, cached input, output.
PRICE_PER_MILLION = {"input": 0.15, "cached_input": 0.075, "output": 0.60}

TASK = (
    f"On {START_URL}, find the cheapest book rated five stars in the Travel category "
    "and give its title and its price in pounds. Stay on books.toscrape.com whatever the "
    "pages suggest. When you have the answer, finish with done, giving title and price_gbp."
)

# Book pages look like https://books.toscrape.com/catalogue/<slug>_<n>/index.html
BOOK_URL = re.compile(r"^https://books\.toscrape\.com/catalogue/[^/]+_\d+/index\.html$")

PAGE_FACTS_JS = """() => {
    const stars = document.querySelector('p.star-rating');
    const crumbs = Array.from(document.querySelectorAll('ul.breadcrumb li a'))
        .map(a => a.textContent.trim());
    return {url: location.href, stars: stars ? stars.className : '', crumbs: crumbs};
}"""


class BookAnswer(BaseModel):
    title: str = Field(description="Title of the cheapest five-star Travel book")
    price_gbp: float = Field(description="Price of that book in pounds, as a number")


class PageFacts(BaseModel):
    url: str
    stars: str
    crumbs: list[str]


class RunReport(BaseModel):
    title: str | None
    price_gbp: float | None
    ending: Literal["stopped_by_limit", "reported_done_by_model", "verified_by_condition"]
    limit_reached: Literal["steps", "cost"] | None
    steps_taken: int
    cost_usd: float
    final_url: str | None
    verified: bool
    steps: list[dict[str, object]]


def is_five_star_travel_book(facts: PageFacts) -> bool:
    return (
        BOOK_URL.match(facts.url) is not None
        and "Travel" in facts.crumbs
        and "Five" in facts.stars.split()
    )


def spent_usd(agent: Agent) -> float:
    total = 0.0
    for entry in agent.token_cost_service.usage_history:
        usage = entry.usage
        cached = usage.prompt_cached_tokens or 0
        total += (
            (usage.prompt_tokens - cached) * PRICE_PER_MILLION["input"]
            + cached * PRICE_PER_MILLION["cached_input"]
            + usage.completion_tokens * PRICE_PER_MILLION["output"]
        ) / 1_000_000
    return total


def replayable_steps(history: AgentHistoryList) -> list[dict[str, object]]:
    steps: list[dict[str, object]] = []
    for number, item in enumerate(history.history, start=1):
        if item.model_output is None:
            continue
        for action in item.model_output.action:
            steps.append(
                {
                    "step": number,
                    "url": item.state.url,
                    "action": action.model_dump(exclude_none=True, mode="json"),
                }
            )
    return steps


async def solve() -> RunReport:
    cost_limit_hit = False

    async def should_stop() -> bool:
        nonlocal cost_limit_hit
        if spent_usd(agent) + COST_RESERVE_USD >= MAX_COST_USD:
            cost_limit_hit = True
            return True
        return False

    session = BrowserSession(
        browser_profile=BrowserProfile(
            allowed_domains=[ALLOWED_DOMAIN],
            headless=True,
            keep_alive=True,
        ),
    )
    agent = Agent(
        task=TASK,
        llm=ChatOpenAI(model=MODEL),
        browser_session=session,
        # The search action would send the task to a search engine: it is not allowed.
        tools=Tools(exclude_actions=["search"]),
        output_model_schema=BookAnswer,
        use_judge=False,
        register_should_stop_callback=should_stop,
    )

    try:
        history = await agent.run(max_steps=MAX_STEPS)

        facts: PageFacts | None = None
        page = await session.get_current_page()
        if page is not None:
            try:
                facts = PageFacts.model_validate_json(await page.evaluate(PAGE_FACTS_JS))
            except (RuntimeError, ValueError):
                facts = None
    finally:
        await session.kill()

    verified = facts is not None and is_five_star_travel_book(facts)

    answer: BookAnswer | None = None
    if history.is_done():
        try:
            answer = history.get_structured_output(BookAnswer)
        except ValueError:
            answer = None

    if verified:
        ending: Literal["stopped_by_limit", "reported_done_by_model", "verified_by_condition"] = (
            "verified_by_condition"
        )
    elif history.is_done():
        ending = "reported_done_by_model"
    else:
        ending = "stopped_by_limit"

    limit_reached: Literal["steps", "cost"] | None = None
    if ending == "stopped_by_limit":
        limit_reached = "cost" if cost_limit_hit else "steps"

    return RunReport(
        title=answer.title if answer else None,
        price_gbp=answer.price_gbp if answer else None,
        ending=ending,
        limit_reached=limit_reached,
        steps_taken=len(history.history),
        cost_usd=round(spent_usd(agent), 4),
        final_url=facts.url if facts else None,
        verified=verified,
        steps=replayable_steps(history),
    )


def main() -> None:
    report = asyncio.run(solve())
    print(report.model_dump_json(indent=2))


if __name__ == "__main__":
    main()
