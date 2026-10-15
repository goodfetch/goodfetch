"""Collects every quote (text, author and tags) from the three versions of quotes.toscrape.com.

The plain version is fetched over HTTP. The JavaScript and infinite-scroll versions need a browser, so their labels force
the browser tier. One handler serves all three labels: the plain and JavaScript pages read their `.quote` elements, and the
scroll version reads the JSON responses its page loads. Each version's pagination is followed, and each quote is returned
once per (version, text, author).
"""

from collections.abc import Mapping, Sequence
from datetime import timedelta

from pydantic import BaseModel

from goodfetch_foxhound import (
    Crawl,
    CrawlContext,
    CrawlRequestSucceeded,
    Dataset,
    Foxhound,
    JSONValue,
    LabelSettings,
    Page,
    Route,
    seed,
)

PLAIN_URL = "https://quotes.toscrape.com/"
JS_URL = "https://quotes.toscrape.com/js/"
SCROLL_URL = "https://quotes.toscrape.com/scroll"

PLAIN_VERSION = "plain"
JS_VERSION = "js"
SCROLL_VERSION = "scroll"

_QUOTE = ".quote"
_QUOTE_TEXT = "span.text"
_QUOTE_AUTHOR = "small.author"
_QUOTE_TAG = ".tags a.tag"
_NEXT_PAGE = "li.next a"
_HEAVY_RESOURCES = frozenset({"image", "font"})
_WAIT = timedelta(seconds=60)
_SCROLL_DELTA_PX = 10_000


class Quote(BaseModel):
    version: str
    text: str
    author: str
    tags: list[str]


class CrawlIncompleteError(RuntimeError):
    """Some crawl requests did not succeed, so the quotes may be incomplete."""


PLAIN_SETTINGS: LabelSettings = {"fetch": {"tier": "http"}}
BROWSER_SETTINGS: LabelSettings = {"fetch": {"tier": "browser"}}

CRAWL = Crawl()
QUOTES: Dataset[Quote] = CRAWL.dataset("quotes", schema=Quote, key=["version", "text", "author"])
PLAIN_LABEL = CRAWL.label(PLAIN_VERSION, settings=PLAIN_SETTINGS)
JS_LABEL = CRAWL.label(JS_VERSION, settings=BROWSER_SETTINGS)
SCROLL_LABEL = CRAWL.label(SCROLL_VERSION, settings=BROWSER_SETTINGS)
SEEDS = [seed(PLAIN_URL, PLAIN_LABEL), seed(JS_URL, JS_LABEL), seed(SCROLL_URL, SCROLL_LABEL)]


def _block_images_and_fonts(route: Route) -> None:
    if route.request.resource_type in _HEAVY_RESOURCES:
        route.abort()
    else:
        route.continue_()


def _is_quotes_api(url: str) -> bool:
    return "/api/quotes" in url


def _as_object(value: JSONValue) -> Mapping[str, JSONValue]:
    return value if isinstance(value, Mapping) else {}


def _author_name(value: JSONValue | None) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, Mapping):
        name = value.get("name")
        if isinstance(name, str):
            return name
    return ""


def _strings(value: JSONValue | None) -> list[str]:
    if not isinstance(value, Sequence) or isinstance(value, str):
        return []
    return [item for item in value if isinstance(item, str)]


def _quotes_from_api(payload: Mapping[str, JSONValue], version: str) -> list[Quote]:
    items = payload.get("quotes")
    if not isinstance(items, Sequence) or isinstance(items, str):
        return []
    quotes: list[Quote] = []
    for item in items:
        if not isinstance(item, Mapping):
            continue
        text = item.get("text")
        if isinstance(text, str):
            quotes.append(
                Quote(
                    version=version,
                    text=text,
                    author=_author_name(item.get("author")),
                    tags=_strings(item.get("tags")),
                )
            )
    return quotes


def _dom_quotes(page: Page, version: str) -> list[Quote]:
    quotes: list[Quote] = []
    for element in page.locator(_QUOTE).all():
        quotes.append(
            Quote(
                version=version,
                text=element.locator(_QUOTE_TEXT).text_content(),
                author=element.locator(_QUOTE_AUTHOR).text_content(),
                tags=element.locator(_QUOTE_TAG).all_text_contents(),
            )
        )
    return quotes


def _scroll_quotes(page: Page, version: str) -> list[Quote]:
    """Reads the quotes from the JSON responses the page loads: the first page on load, then one per scroll."""
    with page.expect_response(_is_quotes_api, timeout=_WAIT) as first:
        page.reload()
    payload = _as_object(first.value.json(timeout=_WAIT))
    quotes: list[Quote] = []
    while True:
        quotes.extend(_quotes_from_api(payload, version))
        if payload.get("has_next") is not True:
            return quotes
        with page.expect_response(_is_quotes_api, timeout=_WAIT) as more:
            page.mouse.wheel(0, _SCROLL_DELTA_PX)
        payload = _as_object(more.value.json(timeout=_WAIT))


def handle_quotes_page(ctx: CrawlContext[None]) -> None:
    version = ctx.request.label.name
    page = ctx.page
    if ctx.tier == "browser":
        page.route("**/*", _block_images_and_fonts)
    if version == SCROLL_VERSION:
        quotes = _scroll_quotes(page, version)
    else:
        if ctx.tier == "browser":
            page.reload()
            page.locator(_QUOTE).first.wait_for(state="attached")
        quotes = _dom_quotes(page, version)
    if not quotes:
        ctx.retry(f"no quotes on {ctx.request.url}")
    for quote in quotes:
        ctx.push(QUOTES, quote)
    if version != SCROLL_VERSION:
        ctx.enqueue_links(page.locator(_NEXT_PAGE), ctx.request.label)


for label in (PLAIN_LABEL, JS_LABEL, SCROLL_LABEL):
    CRAWL.handle(label)(handle_quotes_page)


def collect_quotes() -> list[Quote]:
    with Foxhound.connect() as foxhound:
        run = foxhound.start_crawl(CRAWL, seeds=SEEDS)
        seen: set[tuple[str, str, str]] = set()
        quotes: list[Quote] = []
        for record in run.records(QUOTES):
            quote = record.value
            key = (quote.version, quote.text, quote.author)
            if key not in seen:
                seen.add(key)
                quotes.append(quote)
        unsuccessful = [
            outcome.request.url for outcome in run.outcomes() if not isinstance(outcome, CrawlRequestSucceeded)
        ]
    if unsuccessful:
        raise CrawlIncompleteError(f"{len(unsuccessful)} crawl requests did not succeed: {', '.join(unsuccessful)}")
    return quotes


def main() -> None:
    for quote in collect_quotes():
        print(quote.model_dump_json())


if __name__ == "__main__":
    main()
