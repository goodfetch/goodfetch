"""Scenario 10 on Scrapy 2.19.0 and pytest 9.1.1: tests that run with no network and no model.

Run with `pytest s10_offline_crawl_tests_scrapy.py` from this folder.

* A unit test feeds one saved book page to `BooksSpider.parse_book` and checks the item, including the category.
* A crawl test runs the real `BooksSpider` through Scrapy's engine. A downloader middleware answers every request
  from `RECORDING` (trimmed copies of the pages one live run saw), so the dataset is the real feed export.
* The AI extraction tests run from what an earlier run learned (CSS selectors) or from a scripted model reply, and
  a model that is called fails the test. Scenario 08's program is not given here, so `learn_selectors` and
  `extract` below stand for its two steps: ask the model once for selectors, then extract with selectors alone.
* Every test runs under a guard that refuses any socket connection or DNS lookup of a remote name, and the test
  fails afterwards naming each address that was tried.
"""

import json
import socket
import urllib.request
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass, field
from decimal import Decimal
from pathlib import Path
from typing import Any, NoReturn, Protocol

import pytest
from scrapy import Request, Selector
from scrapy.crawler import CrawlerProcess
from scrapy.http import HtmlResponse, Response, TextResponse
from scrapy.settings import Settings

from s05_two_page_crawl_scrapy import BooksSpider

# ---------------------------------------------------------------------------------------------------------------
# The guard: no test may reach the network.
# ---------------------------------------------------------------------------------------------------------------

LOCAL_NAMES = (None, "", "localhost", "127.0.0.1", "::1")


class OfflineViolation(AssertionError):
    """Raised, and recorded, when code under test tries to reach an address."""


def describe(address: object) -> str:
    if isinstance(address, tuple) and len(address) >= 2:
        return f"{address[0]}:{address[1]}"
    return str(address)


class NetworkGuard:
    def __init__(self) -> None:
        self.attempts: list[str] = []

    def refuse(self, address: object) -> NoReturn:
        text = describe(address)
        self.attempts.append(text)
        raise OfflineViolation(f"offline test tried to reach {text}")

    def install(self, patch: pytest.MonkeyPatch) -> None:
        original_connect: Callable[..., Any] = socket.socket.connect
        original_connect_ex: Callable[..., Any] = socket.socket.connect_ex

        def connect(sock: socket.socket, address: Any, *rest: Any) -> Any:
            if sock.family == socket.AF_UNIX:
                return original_connect(sock, address, *rest)
            self.refuse(address)

        def connect_ex(sock: socket.socket, address: Any, *rest: Any) -> Any:
            if sock.family == socket.AF_UNIX:
                return original_connect_ex(sock, address, *rest)
            self.refuse(address)

        def create_connection(address: Any, *rest: Any, **named: Any) -> NoReturn:
            self.refuse(address)

        def getaddrinfo(host: Any, port: Any, *rest: Any, **named: Any) -> Any:
            if host in LOCAL_NAMES:
                return original_getaddrinfo(host, port, *rest, **named)
            self.refuse((host, port))

        def gethostbyname(host: str) -> Any:
            if host in LOCAL_NAMES:
                return original_gethostbyname(host)
            self.refuse((host, None))

        original_getaddrinfo = socket.getaddrinfo
        original_gethostbyname = socket.gethostbyname
        patch.setattr(socket.socket, "connect", connect)
        patch.setattr(socket.socket, "connect_ex", connect_ex)
        patch.setattr(socket, "create_connection", create_connection)
        patch.setattr(socket, "getaddrinfo", getaddrinfo)
        patch.setattr(socket, "gethostbyname", gethostbyname)

    @contextmanager
    def expect_refusals(self) -> Iterator[list[str]]:
        """Inside the block, refused addresses are expected: they are listed in the yielded list once the block
        ends and do not fail the test."""
        before = len(self.attempts)
        seen: list[str] = []
        try:
            yield seen
        finally:
            seen.extend(self.attempts[before:])
            del self.attempts[before:]


@contextmanager
def no_network() -> Iterator[NetworkGuard]:
    guard = NetworkGuard()
    with pytest.MonkeyPatch.context() as patch:
        guard.install(patch)
        yield guard


@pytest.fixture(autouse=True)
def network_guard() -> Iterator[NetworkGuard]:
    with no_network() as guard:
        yield guard
    assert guard.attempts == [], "offline test tried to reach " + ", ".join(guard.attempts)


# ---------------------------------------------------------------------------------------------------------------
# The saved book page and the recording of one earlier live run.
# ---------------------------------------------------------------------------------------------------------------

SITE = "https://books.toscrape.com/"
BOOK_URL = SITE + "catalogue/a-light-in-the-attic_1000/index.html"

# A saved copy of one book page (trimmed to the markup the spider reads).
SAVED_BOOK_PAGE = """<!DOCTYPE html>
<html lang="en-us">
<head><title>A Light in the Attic | Books to Scrape - Sandbox</title></head>
<body>
<div class="page_inner"><article class="product_page">
<div class="row">
<div class="col-sm-6 product_main">
<h1>A Light in the Attic</h1>
<p class="price_color">£51.77</p>
<p class="instock availability">
    <i class="icon-ok"></i>
    In stock (22 available)
</p>
<p class="star-rating Three">
    <i class="icon-star"></i><i class="icon-star"></i><i class="icon-star"></i>
    <i class="icon-star"></i><i class="icon-star"></i>
</p>
</div>
</div>
</article></div>
</body>
</html>
"""

SAVED_PAGE_WITHOUT_PRICE = SAVED_BOOK_PAGE.replace("£51.77", "")


def book_page(title: str, price: str, stock: int, rating: str) -> str:
    return (
        '<html><body><div class="product_main">'
        f"<h1>{title}</h1>"
        f'<p class="price_color">{price}</p>'
        f'<p class="instock availability"><i class="icon-ok"></i>\n In stock ({stock} available)\n</p>'
        f'<p class="star-rating {rating}"><i class="icon-star"></i></p>'
        "</div></body></html>"
    )


def category_page(slugs: list[str], next_href: str | None) -> str:
    pods = "".join(
        f'<article class="product_pod"><h3><a href="../../../{slug}/index.html">{slug}</a></h3></article>'
        for slug in slugs
    )
    nxt = f'<li class="next"><a href="{next_href}">next</a></li>' if next_href else ""
    return f"<html><body><section>{pods}</section><ul class=\"pager\">{nxt}</ul></body></html>"


def home_page(categories: dict[str, str]) -> str:
    items = "".join(f'<li><a href="{href}"> {name} </a></li>' for name, href in categories.items())
    return (
        '<html><body><div class="side_categories"><ul class="nav nav-list"><li>'
        f'<a href="catalogue/category/books_1/index.html">Books</a><ul>{items}</ul></li></ul></div></body></html>'
    )


TRAVEL = "catalogue/category/books/travel_2/"
MYSTERY = "catalogue/category/books/mystery_3/"

# slug -> (title, price, stock, rating word); "missing-price_4" is a page without a price, which the spider skips.
BOOKS: dict[str, tuple[str, str, int, str]] = {
    "the-himalayas-trip_981": ("The Himalayas Trip", "£45.17", 19, "Two"),
    "full-moon-over-the-ark_811": ("Full Moon over the Ark", "£49.43", 15, "Four"),
    "see-america_489": ("See America", "£48.87", 15, "Three"),
    "sharp-objects_997": ("Sharp Objects", "£47.82", 20, "Four"),
    "in-a-dark-wood_963": ("In a Dark Wood", "£19.63", 19, "One"),
}
BROKEN_SLUG = "missing-price_4"
EXPECTED_BOOK_COUNT = len(BOOKS)  # 5 books are in the dataset; the page without a price is skipped.


def build_recording() -> dict[str, str]:
    recording = {
        SITE + "robots.txt": "User-agent: *\nDisallow: /private/\n",
        SITE: home_page({"Travel": TRAVEL + "index.html", "Mystery": MYSTERY + "index.html"}),
        SITE + TRAVEL + "index.html": category_page(
            ["the-himalayas-trip_981", "full-moon-over-the-ark_811"], "page-2.html"
        ),
        SITE + TRAVEL + "page-2.html": category_page(["see-america_489"], None),
        SITE + MYSTERY + "index.html": category_page(
            ["sharp-objects_997", "in-a-dark-wood_963", BROKEN_SLUG], None
        ),
        SITE + f"catalogue/{BROKEN_SLUG}/index.html": book_page("A Page Without a Price", "", 3, "Five"),
    }
    for slug, (title, price, stock, rating) in BOOKS.items():
        recording[SITE + f"catalogue/{slug}/index.html"] = book_page(title, price, stock, rating)
    return recording


RECORDING = build_recording()


class ReplayMiddleware:
    """Answers every request from `RECORDING`; a request it has no recording for is remembered and fails."""

    unrecorded: list[str] = []

    def process_request(self, request: Request) -> Response | None:
        body = RECORDING.get(request.url)
        if body is None:
            ReplayMiddleware.unrecorded.append(request.url)
            raise LookupError(f"no recorded response for {request.url}")
        cls = TextResponse if request.url.endswith("robots.txt") else HtmlResponse
        return cls(url=request.url, status=200, body=body.encode("utf-8"), encoding="utf-8", request=request)


# ---------------------------------------------------------------------------------------------------------------
# 1. The book-page handler, fed one saved page.
# ---------------------------------------------------------------------------------------------------------------


def saved_response(body: str, url: str = BOOK_URL) -> HtmlResponse:
    return HtmlResponse(url=url, body=body.encode("utf-8"), encoding="utf-8", request=Request(url))


def test_book_page_handler_produces_the_item_with_the_category_it_was_given() -> None:
    spider = BooksSpider()

    items = list(spider.parse_book(saved_response(SAVED_BOOK_PAGE), category="Poetry"))

    assert items == [
        {
            "title": "A Light in the Attic",
            "price": Decimal("51.77"),
            "in_stock": 22,
            "rating": 3,
            "url": BOOK_URL,
            "category": "Poetry",
        }
    ]
    assert spider.skips == []


def test_book_page_without_a_price_yields_no_item_and_is_recorded_as_skipped() -> None:
    spider = BooksSpider()

    items = list(spider.parse_book(saved_response(SAVED_PAGE_WITHOUT_PRICE), category="Poetry"))

    assert items == []
    assert spider.skips == [(BOOK_URL, "no price, stock or rating on the page")]


def test_category_page_hands_its_name_to_the_book_handler() -> None:
    spider = BooksSpider()
    url = SITE + TRAVEL + "index.html"
    response = saved_response(RECORDING[url], url)

    requests = list(spider.parse_category(response, category="Travel"))

    books = [r for r in requests if r.callback == spider.parse_book]
    pages = [r for r in requests if r.callback == spider.parse_category]
    assert [r.url for r in books] == [
        SITE + "catalogue/the-himalayas-trip_981/index.html",
        SITE + "catalogue/full-moon-over-the-ark_811/index.html",
    ]
    assert [r.url for r in pages] == [SITE + TRAVEL + "page-2.html"]
    assert all(r.cb_kwargs == {"category": "Travel"} for r in requests)


# ---------------------------------------------------------------------------------------------------------------
# 2. The whole crawl, from the recorded responses.
# ---------------------------------------------------------------------------------------------------------------


@dataclass
class CrawlResult:
    items: list[dict[str, Any]]
    spider: BooksSpider
    scraped: int
    unrecorded: list[str] = field(default_factory=list[str])


@pytest.fixture(scope="module")
def crawl(tmp_path_factory: pytest.TempPathFactory) -> CrawlResult:
    """Runs the crawl once: Twisted's reactor cannot be restarted within a process."""
    feed: Path = tmp_path_factory.mktemp("feed") / "books.json"
    settings = Settings()
    settings.setdict(
        {
            "FEEDS": {str(feed): {"format": "json", "encoding": "utf8", "overwrite": True}},
            "DOWNLOADER_MIDDLEWARES": {ReplayMiddleware: 900},
            "DOWNLOAD_DELAY": 0,
            "TELNETCONSOLE_ENABLED": False,
            "LOG_LEVEL": "ERROR",
        },
        priority="cmdline",
    )
    ReplayMiddleware.unrecorded.clear()
    with no_network() as guard:
        process = CrawlerProcess(settings)
        crawler = process.create_crawler(BooksSpider)
        process.crawl(crawler)
        process.start(install_signal_handlers=False)
    assert guard.attempts == [], "the crawl tried to reach " + ", ".join(guard.attempts)
    spider = crawler.spider
    assert isinstance(spider, BooksSpider)
    items: list[dict[str, Any]] = json.loads(feed.read_text(encoding="utf-8"))
    scraped = crawler.stats.get_value("item_scraped_count", 0) if crawler.stats else 0
    return CrawlResult(items, spider, scraped, list(ReplayMiddleware.unrecorded))


def test_crawl_from_recorded_responses_holds_the_expected_number_of_books(crawl: CrawlResult) -> None:
    assert crawl.unrecorded == [], "the crawl asked for pages that were never recorded: " + ", ".join(crawl.unrecorded)
    assert len(crawl.items) == EXPECTED_BOOK_COUNT
    assert crawl.scraped == EXPECTED_BOOK_COUNT
    assert len({item["url"] for item in crawl.items}) == EXPECTED_BOOK_COUNT


def test_crawl_gives_each_book_the_category_of_the_page_it_was_reached_from(crawl: CrawlResult) -> None:
    by_category: dict[str, list[str]] = {}
    for item in crawl.items:
        by_category.setdefault(item["category"], []).append(item["title"])

    assert {k: sorted(v) for k, v in by_category.items()} == {
        "Travel": ["Full Moon over the Ark", "See America", "The Himalayas Trip"],  # See America is on page 2
        "Mystery": ["In a Dark Wood", "Sharp Objects"],
    }
    sharp = next(item for item in crawl.items if item["title"] == "Sharp Objects")
    assert sharp["price"] == "47.82" and sharp["in_stock"] == 20 and sharp["rating"] == 4


def test_crawl_skips_the_page_without_a_price_and_reports_no_failure(crawl: CrawlResult) -> None:
    assert crawl.spider.failures == []
    assert crawl.spider.skips == [
        (SITE + f"catalogue/{BROKEN_SLUG}/index.html", "no price, stock or rating on the page")
    ]


# ---------------------------------------------------------------------------------------------------------------
# 3. The AI extraction: from what was learned, or from a scripted reply; never from a real model.
# ---------------------------------------------------------------------------------------------------------------

MODEL_URL = "https://ai-gateway.vercel.sh/v1/chat/completions"
FIELDS = ("title", "price", "rating")

# What the earlier live run learned: one CSS selector per field.
LEARNED_SELECTORS = {
    "title": "h1::text",
    "price": ".price_color::text",
    "rating": ".star-rating::attr(class)",
}
EXPECTED_EXTRACTION = {"title": "A Light in the Attic", "price": "£51.77", "rating": "star-rating Three"}


class Model(Protocol):
    def __call__(self, prompt: str) -> str: ...


def learn_selectors(html: str, model: Model) -> dict[str, str]:
    """One model call: ask for a CSS selector per field of the page."""
    reply = model(f"Give a JSON object with a CSS selector for each of {', '.join(FIELDS)} on this page:\n{html}")
    parsed = json.loads(reply)
    if not isinstance(parsed, dict) or set(parsed) != set(FIELDS):
        raise ValueError(f"the model reply is not a selector for each of {FIELDS}: {reply}")
    return {name: str(css) for name, css in parsed.items()}


def extract(html: str, selectors: dict[str, str]) -> dict[str, str]:
    page = Selector(text=html)
    return {name: (page.css(css).get() or "").strip() for name, css in selectors.items()}


def extract_book(html: str, model: Model, learned: dict[str, str] | None) -> dict[str, str]:
    """Extract with what was learned when there is any; otherwise learn first, which calls the model once."""
    return extract(html, learned if learned is not None else learn_selectors(html, model))


@dataclass
class ScriptedModel:
    replies: list[str]
    prompts: list[str] = field(default_factory=list[str])

    def __call__(self, prompt: str) -> str:
        self.prompts.append(prompt)
        if not self.replies:
            raise AssertionError("the scripted model was asked more often than it has replies")
        return self.replies.pop(0)


@dataclass
class ForbiddenModel:
    """A model that fails the test when it is called; `calls` also shows it if the code under test swallows the error."""

    calls: list[str] = field(default_factory=list[str])

    def __call__(self, prompt: str) -> str:
        self.calls.append(prompt)
        raise AssertionError("a real model was called")


class HttpModel:
    """A real model client over HTTP: here only to prove the guard stops it."""

    def __init__(self, url: str) -> None:
        self.url = url

    def __call__(self, prompt: str) -> str:
        request = urllib.request.Request(
            self.url, data=json.dumps({"prompt": prompt}).encode(), headers={"Content-Type": "application/json"}
        )
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(request, timeout=5) as reply:
            return reply.read().decode()


def test_extraction_from_what_was_learned_never_calls_the_model() -> None:
    model = ForbiddenModel()

    result = extract_book(SAVED_BOOK_PAGE, model, LEARNED_SELECTORS)

    assert result == EXPECTED_EXTRACTION
    assert model.calls == []


def test_extraction_from_a_scripted_model_reply_calls_it_once_and_learns_the_selectors() -> None:
    model = ScriptedModel([json.dumps(LEARNED_SELECTORS)])

    first = extract_book(SAVED_BOOK_PAGE, model, None)
    learned = learn_selectors(SAVED_BOOK_PAGE, ScriptedModel([json.dumps(LEARNED_SELECTORS)]))
    second = extract_book(SAVED_BOOK_PAGE, model, learned)

    assert learned == LEARNED_SELECTORS
    assert first == second == EXPECTED_EXTRACTION
    assert len(model.prompts) == 1
    assert "A Light in the Attic" in model.prompts[0]
    assert model.replies == []


def test_a_scripted_reply_that_is_not_a_selector_per_field_is_refused() -> None:
    model = ScriptedModel([json.dumps({"title": "h1::text"})])

    with pytest.raises(ValueError, match="not a selector for each of"):
        learn_selectors(SAVED_BOOK_PAGE, model)


def test_a_real_model_call_is_refused_and_names_the_address_it_tried(network_guard: NetworkGuard) -> None:
    with network_guard.expect_refusals() as seen, pytest.raises(OfflineViolation, match="ai-gateway.vercel.sh"):
        extract_book(SAVED_BOOK_PAGE, HttpModel(MODEL_URL), None)

    assert seen and all(address.startswith("ai-gateway.vercel.sh") for address in seen)


# ---------------------------------------------------------------------------------------------------------------
# The guard itself: a test that would reach the network fails, saying which address.
# ---------------------------------------------------------------------------------------------------------------


def test_guard_names_the_address_of_a_connection(network_guard: NetworkGuard) -> None:
    with network_guard.expect_refusals() as seen, pytest.raises(OfflineViolation, match=r"203\.0\.113\.5:443"):
        socket.create_connection(("203.0.113.5", 443))

    assert seen == ["203.0.113.5:443"]


def test_guard_names_the_host_of_a_name_lookup(network_guard: NetworkGuard) -> None:
    with network_guard.expect_refusals() as seen, pytest.raises(OfflineViolation, match=r"books\.toscrape\.com"):
        socket.getaddrinfo("books.toscrape.com", 443)

    assert seen == ["books.toscrape.com:443"]


def test_guard_makes_a_test_fail_when_the_error_is_swallowed() -> None:
    """The check pytest runs after each test: addresses tried inside code that hides the error still fail it."""
    with no_network() as guard:
        try:
            socket.create_connection(("books.toscrape.com", 443))
        except OfflineViolation:
            pass  # swallowed, as a crawler's errback might

    assert guard.attempts == ["books.toscrape.com:443"]
