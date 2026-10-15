"""Scenario 03: plain-words AI steps, kept in a Plan and replayed without a model (ADR 0091).

On quotes.toscrape.com the program opens the "love" tag, goes to its second page and opens the about page of the first
quote's author, then returns the author's name, birth date and birthplace. Each step is written in words. The model
resolves a step only when the Plan file next to this program holds no entry for it; a later run replays every kept step
with no model call. A kept step that no longer fits the site goes back to the model alone, and the program names it.
"""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass
from datetime import timedelta
from pathlib import Path
from typing import TypeVar

from pydantic import BaseModel, Field

from goodfetch_collie import Collie, RunSettings, StepResult, expect

START_URL = "https://quotes.toscrape.com/"
MODEL = "vercel/anthropic/claude-sonnet-4.5"
PLAN_FILE = Path(__file__).resolve().with_name("s03_replay_ai_steps.plan.json")
RUN_TIMEOUT = timedelta(minutes=10)
RUN_SETTINGS: RunSettings = {"ai": {"model": MODEL}}

OPEN_TAG = 'open the tag "love" from the tag list'
OPEN_SECOND_PAGE = "go to the second page of this tag's quotes"
OPEN_FIRST_AUTHOR_ABOUT = 'open the "(about)" link of the author of the first quote on this page'
READ_AUTHOR = "the author's name, birth date and birthplace on this author page"

TAG_LOVE_URL = re.compile(r"^https://quotes\.toscrape\.com/tag/love/?$")
TAG_LOVE_PAGE_2_URL = re.compile(r"^https://quotes\.toscrape\.com/tag/love/page/2/?$")
AUTHOR_PAGE_URL = re.compile(r"^https://quotes\.toscrape\.com/author/")

LOG = logging.getLogger(__name__)
_T = TypeVar("_T")


class Author(BaseModel):
    """The facts an author page states."""

    name: str = Field(description="The author's full name, as the page states it")
    birth_date: str = Field(description="The author's birth date, as the page states it")
    birthplace: str = Field(description="The place the author was born, as the page states it")


@dataclass(frozen=True, kw_only=True)
class Outcome:
    """What the scenario returns: the author's facts and how each step was resolved."""

    name: str
    birth_date: str
    birthplace: str
    author_page: str
    verdict: str
    plan_status: str
    step_sources: tuple[str, ...]
    repaired_steps: tuple[str, ...]


class _Trail:
    """Records each step's source, and the steps that went back to the model after drift."""

    def __init__(self) -> None:
        self.sources: list[str] = []
        self.repaired: list[str] = []

    def note(self, result: StepResult[_T]) -> None:
        step = result.step
        self.sources.append(f"{step.source}: {step.instruction}")
        if step.source == "repair":
            self.repaired.append(step.instruction)
            LOG.warning("step went back to the model after drift: %s (drift: %s)", step.instruction, step.drift)


def run_steps(collie: Collie) -> Outcome:
    """Runs the scenario once in a new Run that uses the shared Plan file."""
    trail = _Trail()
    run = collie.start(plan=PLAN_FILE, settings=RUN_SETTINGS, timeout=RUN_TIMEOUT)
    try:
        page = run.new_page()
        page.goto(START_URL)
        trail.note(page.act(OPEN_TAG))
        expect(page).to_have_url(TAG_LOVE_URL)
        trail.note(page.act(OPEN_SECOND_PAGE))
        expect(page).to_have_url(TAG_LOVE_PAGE_2_URL)
        trail.note(page.act(OPEN_FIRST_AUTHOR_ABOUT))
        expect(page).to_have_url(AUTHOR_PAGE_URL)
        author_page = page.url
        facts = page.extract(READ_AUTHOR, Author, mode="compiled")
        trail.note(facts)
        report = run.finish()
    finally:
        run.close()

    return Outcome(
        name=facts.output.name,
        birth_date=facts.output.birth_date,
        birthplace=facts.output.birthplace,
        author_page=author_page,
        verdict=report.verdict,
        plan_status=report.plan.status,
        step_sources=tuple(trail.sources),
        repaired_steps=tuple(trail.repaired),
    )


def main() -> Outcome:
    logging.basicConfig(level=logging.INFO)
    with Collie.connect() as collie:
        outcome = run_steps(collie)
    print(f"name: {outcome.name}")
    print(f"birth date: {outcome.birth_date}")
    print(f"birthplace: {outcome.birthplace}")
    print(f"author page: {outcome.author_page}")
    print(f"verdict: {outcome.verdict}, plan: {outcome.plan_status}")
    for source in outcome.step_sources:
        print(f"step {source}")
    if outcome.repaired_steps:
        for instruction in outcome.repaired_steps:
            print(f"went back to the model: {instruction}")
    else:
        print("no step went back to the model")
    return outcome


if __name__ == "__main__":
    main()
