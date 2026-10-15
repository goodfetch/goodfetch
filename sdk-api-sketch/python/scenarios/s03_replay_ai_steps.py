"""Golden scenario 03, "Repeat AI steps with no model call" (ADRs 0091, 0101, 0102)."""

import asyncio
import re
from pathlib import Path

from pydantic import BaseModel

from goodfetch_collie import AsyncCollie, expect


class Author(BaseModel):
    name: str
    birth_date: str
    birthplace: str


STEPS = (
    'open the tag "love" from the tag list',
    "go to its second page",
    "open the about page of the first quote's author",
)


async def main() -> None:
    # The model is called only for a step the Plan file lacks or a step that drifted (ADR 0091). One entry per call,
    # kept next to the program; a new version is written only after a `verified` Run.
    async with (
        AsyncCollie.connect(
            settings={"ai": {"model": "anthropic/claude-sonnet-4-5"}}
        ) as collie,
        await collie.start(plan=Path(__file__).with_name("quotes.plan.json")) as run,
    ):
        page = await run.new_page()
        await page.goto("https://quotes.toscrape.com/")
        for instruction in STEPS:
            # Replayed with no model call when its Target fingerprint still matches; on Drift, this step alone gets one
            # Repair, and a Drift left (no model, `plan.frozen`) raises PlanDriftError naming it.
            step = (await page.act(instruction)).step
            if step.source == "repair":
                print(f'The site changed: "{step.instruction}" went back to the model.')
        # A Success check without a model: the Run is `verified`, and its Plan kept, only if it holds.
        await expect(page).to_have_url(
            re.compile(r"^https://quotes\.toscrape\.com/author/")
        )
        # Compiled by default: the model writes each field's locator once, then values are copied from the page.
        author = (
            await page.extract("the author's name, birth date and birthplace", Author)
        ).output
        report = await run.finish()
        print(author, f"Plan {report.plan.status}", report.plan.changed_steps)


asyncio.run(main())
