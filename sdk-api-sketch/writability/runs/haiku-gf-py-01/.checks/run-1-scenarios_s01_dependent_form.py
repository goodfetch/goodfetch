"""Scenario s01: the dependent form on quotes.toscrape.com.

Chooses the author "Albert Einstein" (the page reloads and then offers that author's tags), chooses the tag
"inspirational", presses Search and returns the quotes shown as typed objects. Everything the program opens is closed
before it returns or raises.
"""

import logging
from collections.abc import Callable
from datetime import timedelta
from typing import TypeVar

from goodfetch_collie import Collie, GoodfetchError, Locator, Page
from pydantic import BaseModel, ConfigDict

SEARCH_URL = "https://quotes.toscrape.com/search.aspx"
AUTHOR = "Albert Einstein"
TAG = "inspirational"
POLLING = timedelta(milliseconds=200)

# Set on the page before the author is chosen. A reload discards it, so its absence means the new document has loaded.
_MARK_PAGE = "() => { window.__goodfetchReloadMarker = true; return true; }"
_PAGE_RELOADED = "() => document.readyState === 'complete' && window.__goodfetchReloadMarker !== true"
_RESULTS_SHOWN = "() => document.querySelector('.quote') !== null || /no quotes found/i.test(document.body.innerText)"

logger = logging.getLogger(__name__)
_T = TypeVar("_T")


class Quote(BaseModel):
    """A quote shown by the search results."""

    model_config = ConfigDict(frozen=True)

    text: str
    author: str
    tag: str


class DependentFormError(Exception):
    """A step of the dependent form did not behave as described."""

    def __init__(self, step: str, observed: str) -> None:
        super().__init__(f"{step} failed: {observed}")
        self.step = step
        self.observed = observed


def find_einstein_inspirational_quotes() -> list[Quote]:
    """Returns the quotes the search shows for the author and the tag, closing everything it opened."""
    with _step("connect to the engine", Collie.connect) as collie:
        with _step("start the run", collie.start) as run:
            with _step("open a page", run.new_page) as page:
                _open_search(page)
                _choose_author(page)
                _choose_tag(page)
                quotes = _search(page)
            _step("finish the run", run.finish)
    return quotes


def _step(step: str, action: Callable[[], _T]) -> _T:
    """Runs one SDK call, naming the step in the error if the SDK refuses it."""
    try:
        return action()
    except GoodfetchError as error:
        observed = error.observed or error.message
        raise DependentFormError(step, f"{error.code}: {observed}") from error


def _open_search(page: Page) -> None:
    response = _step("open the search page", lambda: page.goto(SEARCH_URL))
    if response is not None and not response.ok:
        raise DependentFormError("open the search page", f"HTTP {response.status} from {SEARCH_URL}")


def _choose_author(page: Page) -> None:
    _step("mark the page before the author change", lambda: page.evaluate(_MARK_PAGE))
    _step("choose the author", lambda: page.locator("#author").select_option(label=AUTHOR))
    _step(
        "wait for the page to reload after the author change",
        lambda: page.wait_for_function(_PAGE_RELOADED, polling=POLLING),
    )


def _choose_tag(page: Page) -> None:
    offered = _step("read the tag list", lambda: page.locator("#tag option").all_text_contents())
    tags = [tag.strip() for tag in offered]
    if TAG not in tags:
        raise DependentFormError(
            "choose the tag",
            f"no tag {TAG!r} offered after choosing author {AUTHOR!r}; the tag list was {tags}",
        )
    _step("choose the tag", lambda: page.locator("#tag").select_option(label=TAG))


def _search(page: Page) -> list[Quote]:
    _step("press Search", lambda: page.get_by_role("button", name="Search", exact=True).click())
    _step("wait for the results", lambda: page.wait_for_function(_RESULTS_SHOWN, polling=POLLING))
    nodes = _step("read the results", lambda: page.locator(".quote").all())
    if not nodes:
        logger.warning("No quote matches the author %r and the tag %r; returning an empty list.", AUTHOR, TAG)
        return []
    return [_read_quote(node) for node in nodes]


def _read_quote(node: Locator) -> Quote:
    text = _step("read a quote's text", lambda: node.locator(".text").text_content())
    author = _step("read a quote's author", lambda: node.locator(".author").text_content())
    return Quote(text=text.strip(), author=author.strip(), tag=TAG)


if __name__ == "__main__":
    for quote in find_einstein_inspirational_quotes():
        print(quote.model_dump_json())
