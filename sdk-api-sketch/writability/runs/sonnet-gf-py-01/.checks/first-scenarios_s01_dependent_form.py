"""Scenario 01: a dependent form on quotes.toscrape.com/search.aspx.

Choose the author "Albert Einstein" (the page reloads and offers that author's tags), choose the tag "inspirational",
press Search and return every quote shown.
"""

from __future__ import annotations

from dataclasses import dataclass

from goodfetch_collie import Collie, GoodfetchError, Page

URL = "https://quotes.toscrape.com/search.aspx"
AUTHOR = "Albert Einstein"
TAG = "inspirational"


@dataclass(frozen=True)
class Quote:
    text: str
    author: str
    tag: str


class FormStepError(Exception):
    """The page did not behave as described: names the step and what was seen."""

    def __init__(self, step: str, seen: str) -> None:
        super().__init__(f"step '{step}' failed: {seen}")
        self.step = step
        self.seen = seen


def _is_search_response(url: str) -> bool:
    return url.split("?", 1)[0].endswith("/search.aspx")


def _describe(page: Page) -> str:
    """What the page shows now, for an error message; never raises."""
    try:
        authors = page.locator("#author option").all_text_contents()
        tags = page.locator("#tag option").all_text_contents()
        return f"url {page.url}; authors {authors}; tags {tags}"
    except GoodfetchError as err:
        return f"url {page.url}; page unreadable ({err.message})"


def fetch_quotes() -> list[Quote]:
    with Collie.connect() as collie, collie.start() as run, run.new_page() as page:
        # Step 1: open the page.
        try:
            page.goto(URL)
            page.locator("#author").wait_for()
        except GoodfetchError as err:
            raise FormStepError("open the search page", f"{err.message}; {_describe(page)}") from err

        # Step 2: choose the author; the page reloads and offers that author's tags.
        try:
            with page.expect_response(_is_search_response):
                page.locator("#author").select_option(label=AUTHOR)
            page.wait_for_load_state("load")
            chosen = page.locator("#author").input_value()
        except GoodfetchError as err:
            raise FormStepError(f"choose author {AUTHOR!r}", f"{err.message}; {_describe(page)}") from err
        if chosen != AUTHOR:
            raise FormStepError(f"choose author {AUTHOR!r}", f"author select shows {chosen!r} after reload; {_describe(page)}")

        # Step 3: the tag list now offers the author's tags; choose the tag.
        try:
            page.locator("#tag option").filter(has_text=TAG).first.wait_for(state="attached")
            page.locator("#tag").select_option(label=TAG)
            tag_value = page.locator("#tag").input_value()
        except GoodfetchError as err:
            raise FormStepError(f"choose tag {TAG!r}", f"{err.message}; {_describe(page)}") from err
        if tag_value != TAG:
            raise FormStepError(f"choose tag {TAG!r}", f"tag select shows {tag_value!r}; {_describe(page)}")

        # Step 4: press Search and read the quotes shown.
        try:
            with page.expect_response(_is_search_response):
                page.get_by_role("button", name="Search", exact=True).click()
            page.wait_for_load_state("load")
            quotes: list[Quote] = []
            for item in page.locator(".quote").all():
                text = item.locator(".content").text_content().strip()
                author = item.locator(".author").text_content().strip()
                quotes.append(Quote(text=text, author=author, tag=TAG))
        except GoodfetchError as err:
            raise FormStepError("press Search and read the quotes", f"{err.message}; {_describe(page)}") from err

        run.finish()

    if not quotes:
        print(f"No quote matches author {AUTHOR!r} with tag {TAG!r}: returning an empty list.")
    return quotes


def main() -> None:
    for quote in fetch_quotes():
        print(f"{quote.author} [{quote.tag}]: {quote.text}")


if __name__ == "__main__":
    main()
