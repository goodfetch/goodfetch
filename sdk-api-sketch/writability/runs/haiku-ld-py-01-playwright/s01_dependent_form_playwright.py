"""Return the quotes by Albert Einstein tagged "inspirational" on quotes.toscrape.com."""

from dataclasses import dataclass

from playwright.sync_api import (
    Browser,
    Error as PlaywrightError,
    Locator,
    Page,
    sync_playwright,
)

SEARCH_URL = "https://quotes.toscrape.com/search.aspx"
AUTHOR = "Albert Einstein"
TAG = "inspirational"
TIMEOUT_MS = 15_000


@dataclass(frozen=True)
class Quote:
    text: str
    author: str
    tag: str


class SearchStepError(RuntimeError):
    """A step of the search did not behave as described; names the step and what was seen."""

    def __init__(self, step: str, observed: str) -> None:
        super().__init__(f"Step '{step}' failed: {observed}")
        self.step = step
        self.observed = observed


def _option_labels(select: Locator) -> list[str]:
    return [label.strip() for label in select.locator("option").all_inner_texts()]


def _choose_author(page: Page) -> None:
    author_select = page.locator("#author")
    offered = _option_labels(author_select)
    if AUTHOR not in offered:
        raise SearchStepError(
            "choose author",
            f"author '{AUTHOR}' is not offered; the author list was {offered}",
        )
    try:
        with page.expect_navigation(timeout=TIMEOUT_MS):
            author_select.select_option(label=AUTHOR)
    except PlaywrightError as error:
        raise SearchStepError(
            "choose author",
            f"choosing '{AUTHOR}' did not reload the page ({error})",
        ) from error


def _choose_tag(page: Page) -> None:
    tag_select = page.locator("#tag")
    offered = _option_labels(tag_select)
    author_tags = [label for label in offered if label.strip("-")]
    if not author_tags:
        raise SearchStepError(
            "read the tags offered for the author",
            f"no tags after choosing '{AUTHOR}'; the tag list was {offered}",
        )
    if TAG not in offered:
        raise SearchStepError(
            "choose tag",
            f"tag '{TAG}' is not offered for '{AUTHOR}'; the tag list was {offered}",
        )
    try:
        tag_select.select_option(label=TAG)
    except PlaywrightError as error:
        raise SearchStepError("choose tag", f"could not select '{TAG}' ({error})") from error


def _press_search(page: Page) -> None:
    try:
        with page.expect_navigation(timeout=TIMEOUT_MS):
            page.locator("#submit_button").click()
    except PlaywrightError as error:
        raise SearchStepError(
            "press Search",
            f"pressing Search did not load the results ({error})",
        ) from error


def _read_results(page: Page) -> list[Quote]:
    cards = page.locator("div.quote")
    count = cards.count()
    if count == 0:
        page_text = page.locator("body").inner_text()
        if "no quotes found" in page_text.lower():
            return []
        raise SearchStepError(
            "read results",
            f"no quote cards and no 'No quotes found' message; the page text starts with {page_text[:200]!r}",
        )
    quotes: list[Quote] = []
    for index in range(count):
        card = cards.nth(index)
        try:
            text = card.locator("span.text").inner_text().strip()
            author = card.locator("small.author").inner_text().strip()
        except PlaywrightError as error:
            raise SearchStepError(
                "read results",
                f"quote card {index + 1} has no text or author ({error})",
            ) from error
        quotes.append(Quote(text=text, author=author, tag=TAG))
    return quotes


def _search(browser: Browser) -> list[Quote]:
    context = browser.new_context()
    try:
        page = context.new_page()
        page.set_default_timeout(TIMEOUT_MS)
        try:
            page.goto(SEARCH_URL)
        except PlaywrightError as error:
            raise SearchStepError("open the search page", f"{SEARCH_URL} did not load ({error})") from error
        _choose_author(page)
        _choose_tag(page)
        _press_search(page)
        return _read_results(page)
    finally:
        context.close()


def find_inspirational_einstein_quotes() -> list[Quote]:
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        try:
            return _search(browser)
        finally:
            browser.close()


if __name__ == "__main__":
    for quote in find_inspirational_einstein_quotes():
        print(f"{quote.author} [{quote.tag}]: {quote.text}")
