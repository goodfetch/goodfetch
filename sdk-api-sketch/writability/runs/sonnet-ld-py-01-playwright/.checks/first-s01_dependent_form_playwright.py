"""Search quotes.toscrape.com/search.aspx for one author and one tag."""

from __future__ import annotations

from dataclasses import dataclass

from playwright.sync_api import Page, sync_playwright
from playwright.sync_api import Error as PlaywrightError
from playwright.sync_api import TimeoutError as PlaywrightTimeoutError

URL = "https://quotes.toscrape.com/search.aspx"
AUTHOR = "Albert Einstein"
TAG = "inspirational"
STEP_TIMEOUT_MS = 15_000
RESULT_TIMEOUT_MS = 10_000


class StepError(Exception):
    """The page did not behave as described; says which step and what was seen."""

    def __init__(self, step: str, seen: str) -> None:
        super().__init__(f"step '{step}' failed: {seen}")
        self.step = step
        self.seen = seen


@dataclass(frozen=True)
class Quote:
    text: str
    author: str
    tag: str


def _visible_text(page: Page, selector: str) -> str:
    try:
        return page.locator(selector).first.inner_text(timeout=1_000).strip()
    except PlaywrightError:
        return "<unreadable>"


def _options(page: Page, selector: str) -> list[str]:
    try:
        return [t.strip() for t in page.locator(f"{selector} > option").all_inner_texts()]
    except PlaywrightError:
        return []


def _choose_author(page: Page) -> None:
    step = f"choose author '{AUTHOR}'"
    try:
        page.locator("#author").wait_for(state="attached", timeout=STEP_TIMEOUT_MS)
    except PlaywrightTimeoutError as exc:
        raise StepError(step, "no author drop-down (#author) appeared on the page") from exc
    options = _options(page, "#author")
    if AUTHOR not in options:
        raise StepError(step, f"author options were {options!r}")
    page.select_option("#author", label=AUTHOR, timeout=STEP_TIMEOUT_MS)


def _choose_tag(page: Page) -> None:
    step = f"choose tag '{TAG}' after the author changed"
    try:
        page.locator("#tag > option", has_text=TAG).first.wait_for(
            state="attached", timeout=STEP_TIMEOUT_MS
        )
    except PlaywrightTimeoutError as exc:
        raise StepError(
            step, f"the tag list never offered '{TAG}'; tag options were {_options(page, '#tag')!r}"
        ) from exc
    page.select_option("#tag", label=TAG, timeout=STEP_TIMEOUT_MS)


def _press_search(page: Page) -> None:
    step = "press Search"
    button = page.locator("input[type=submit][value=Search], button:has-text('Search')").first
    try:
        button.click(timeout=STEP_TIMEOUT_MS)
    except PlaywrightTimeoutError as exc:
        raise StepError(step, "no clickable Search button was found") from exc


def _read_quotes(page: Page) -> list[Quote]:
    step = "read results"
    cards = page.locator("div.quote")
    try:
        cards.first.wait_for(state="attached", timeout=RESULT_TIMEOUT_MS)
    except PlaywrightTimeoutError:
        return []
    quotes: list[Quote] = []
    for i in range(cards.count()):
        card = cards.nth(i)
        parts: dict[str, str] = {}
        for name, selector in (("text", ".content"), ("author", ".author"), ("tag", ".tag")):
            loc = card.locator(selector).first
            try:
                parts[name] = loc.inner_text(timeout=STEP_TIMEOUT_MS).strip()
            except PlaywrightTimeoutError as exc:
                raise StepError(step, f"result {i + 1} has no '{selector}' element") from exc
        quotes.append(Quote(parts["text"], parts["author"], parts["tag"]))
    return quotes


def search_quotes() -> list[Quote]:
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        try:
            context = browser.new_context()
            try:
                page = context.new_page()
                page.set_default_timeout(STEP_TIMEOUT_MS)
                try:
                    response = page.goto(URL, wait_until="load")
                except PlaywrightError as exc:
                    raise StepError("open page", f"navigation to {URL} failed: {exc}") from exc
                if response is None or not response.ok:
                    status = "no response" if response is None else f"HTTP {response.status}"
                    raise StepError("open page", f"{URL} answered {status}")
                _choose_author(page)
                _choose_tag(page)
                _press_search(page)
                return _read_quotes(page)
            finally:
                context.close()
        finally:
            browser.close()


def main() -> None:
    quotes = search_quotes()
    if not quotes:
        print(f"No quote matches author '{AUTHOR}' with tag '{TAG}'.")
    for quote in quotes:
        print(f"{quote.text} - {quote.author} [{quote.tag}]")


if __name__ == "__main__":
    main()
