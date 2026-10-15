"""Golden scenario 11, "Reach a goal from one instruction, with limits and a verdict", on Browser Use 0.13.11."""

import asyncio
import os
from dataclasses import dataclass
from typing import Literal

# Telemetry to PostHog is on by default, and cloud sync follows it; both read this before the first run.
os.environ["ANONYMIZED_TELEMETRY"] = "false"

from browser_use import Agent, Browser, ChatAnthropic
from pydantic import BaseModel, Field

MAX_PRICE_USD = 0.20


class Answer(BaseModel):
    title: str
    price: str = Field(pattern=r"^£\d+\.\d{2}$")


@dataclass(frozen=True)
class Outcome:
    ended: Literal["verified", "reported", "stopped"]
    answer: Answer | None
    why: str


async def stop_at_price(agent: Agent) -> None:
    """Browser Use has no price cap: the program checks the spend after each step and stops the agent."""
    usage = await agent.token_cost_service.get_usage_summary()
    if usage.total_cost >= MAX_PRICE_USD:
        agent.stop()


async def travel_book_check(browser: Browser) -> bool:
    """The program's own condition, read on the page the agent left open."""
    page = await browser.get_current_page()
    if page is None:
        return False
    found = await page.evaluate(
        "() => !!document.querySelector('.breadcrumb')?.innerText.includes('Travel')"
        " && !!document.querySelector('.product_main .star-rating.Five')"
    )
    return found == "True"  # `evaluate` returns Python's text for a boolean


async def main() -> None:
    browser = Browser(allowed_domains=["books.toscrape.com"], keep_alive=True)
    agent = Agent(
        task="On https://books.toscrape.com/, find the cheapest book rated five stars in the Travel category"
        " and give its title and price",
        llm=ChatAnthropic(model="claude-sonnet-4-5"),  # reads ANTHROPIC_API_KEY
        browser=browser,
        output_model_schema=Answer,
        calculate_cost=True,  # prices from LiteLLM's table, fetched from GitHub
    )
    try:
        history = await agent.run(max_steps=25, on_step_end=stop_at_price)
        # Kept for `agent.load_and_rerun()`, which still calls a model to re-extract and summarize.
        history.save_to_file("travel.history.json")
        if not history.is_done():
            # The step cap and the failure cap leave their reason as the last step's error text; a stop does not.
            spent = history.usage.total_cost if history.usage else 0.0
            why = (
                "price" if spent >= MAX_PRICE_USD else history.errors()[-1] or "unknown"
            )
            outcome = Outcome(ended="stopped", answer=None, why=why)
        elif await travel_book_check(browser):
            outcome = Outcome(
                ended="verified",
                answer=history.structured_output,
                why="travel_book_check",
            )
        else:
            outcome = Outcome(
                ended="reported",
                answer=history.structured_output,
                why=f"success={history.is_successful()}",
            )
    finally:
        await browser.kill()
    print(outcome)


asyncio.run(main())
