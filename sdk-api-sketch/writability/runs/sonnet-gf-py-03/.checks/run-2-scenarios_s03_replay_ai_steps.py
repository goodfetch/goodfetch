"""Quotes to Scrape: open the tag "love", its second page, the first quote's author page, and read the author.

Each navigation step is written in plain words and resolved through `page.act`. What the model resolved for every
step is kept in a Plan file next to this program. A later run replays the kept steps with no model call; when the
site changes so that a kept step no longer applies, only that step is sent back to the model (a Repair), and the
program reports which step it was.
"""

import json
import re
import sys
from pathlib import Path

from pydantic import BaseModel

from goodfetch_collie import Collie, PlanDriftError, StepReport, expect

START_URL = "https://quotes.toscrape.com/"
AUTHOR_URL = re.compile(r"^https://quotes\.toscrape\.com/author/")
PLAN_FILE = Path(__file__).resolve().with_name("s03_replay_ai_steps.plan.json")

STEPS: tuple[str, ...] = (
    'In the "Top Ten tags" list, click the tag "love"',
    'Go to the second page of the quotes, with the "Next" link',
    'Open the "(about)" page of the author of the first quote',
)
EXTRACT_STEP = "Read the author's name, birth date and birthplace from the author page"


class Author(BaseModel):
    name: str
    birth_date: str
    birthplace: str


def _say(message: str) -> None:
    print(message, file=sys.stderr)


def _note(step: StepReport, number: int, instruction: str) -> int:
    """Reports how a step was resolved; returns 1 when the model was involved, else 0."""
    if step.source == "plan":
        _say(f"step {number} replayed from the plan, no model: {instruction}")
        return 0
    if step.source == "repair":
        drift = step.drift
        detail = "" if drift is None else f" (expected {drift.expected}; found {drift.observed})"
        _say(f"step {number} no longer applied and went back to the model: {instruction}{detail}")
    else:
        _say(f"step {number} resolved by the model ({step.source}): {instruction}")
    return 1


def scrape() -> Author:
    with Collie.connect() as collie:
        with collie.start(plan=str(PLAN_FILE)) as run:
            page = run.new_page()
            page.goto(START_URL)

            model_steps = 0
            for number, instruction in enumerate(STEPS, start=1):
                try:
                    result = page.act(instruction)
                except PlanDriftError:
                    _say(f"step {number} drifted and could not be repaired: {instruction}")
                    raise
                model_steps += _note(result.step, number, instruction)

            # Checked without a model: a URL assertion is evaluated by the engine.
            expect(page).to_have_url(AUTHOR_URL)

            extracted = page.extract(EXTRACT_STEP, Author)
            model_steps += _note(extracted.step, len(STEPS) + 1, EXTRACT_STEP)
            author = extracted.output

            report = run.finish()
            if report.plan.changed_steps:
                _say("steps changed in the plan: " + ", ".join(report.plan.changed_steps))
            if model_steps == 0:
                _say("no model call was made in this run")
            return author


def main() -> None:
    author = scrape()
    print(json.dumps(author.model_dump(), indent=2))


if __name__ == "__main__":
    main()
