"""Offline tests for the scenario 05 crawl (Scrapy 2.19.0, pytest 9.1.1).

Runs with no network and no model. An autouse fixture refuses every socket entry point, and a refused
attempt fails the test with the address it tried.

The saved book page and the recorded responses are hand-written stand-ins shaped like books.toscrape.com
pages. No recording from a live run is in the working folder, so the expected book count comes from these
fixtures, not from a live run.

Scenario 08's program is not in the working folder either, so the AI test drives a scripted model reply
through the same book fields and checks it against the scenario 05 handler. Point it at scenario 08's
extraction function when that program is supplied.
"""

import json
import socket
from collections import deque
from decimal import Decimal
from typing import Any, NoReturn

import pytest
import scrapy
from scrapy.http import HtmlResponse

from s05_two_page_crawl_scrapy import BooksSpider

BASE = "https://books.toscrape.com"
HOME_URL = f"{BASE}/"
POETRY_URL = f"{BASE}/catalogue/category/books/poetry_23/index.html"
POETRY_PAGE_2_URL = f"{BASE}/catalogue/category/books/poetry_23/page-2.html"
TRAVEL_URL = f"{BASE}/catalogue/category/books/travel_2/index.html"
A_LIGHT_URL = f"{BASE}/catalogue/a-light-in-the-attic_1000/index.html"
TIPPING_URL = f"{BASE}/catalogue/tipping-the-velvet_999/index.html"
SOUMISSION_URL = f"{BASE}/catalogue/soumission_998/index.html"
HIMALAYAS_URL = f"{BASE}/catalogue/its-only-the-himalayas_981/index.html"

SAVED_BOOK_PAGE = """<!DOCTYPE html>
<html lang="en-us">
<head><title>A Light in the Attic | Books to Scrape - Sandbox</title></head>
<body>
<article class="product_page">
  <div class="row">
    <div class="col-sm-6 product_main">
      <h1>A Light in the Attic</h1>
      <p class="price_color">£51.77</p>
      <p class="instock availability">
        <i class="icon-ok"></i>
        In stock (22 available)
      </p>
      <p class="star-rating Three"><i class="icon-star"></i></p>
    </div>
  </div>
</article>
</body>
</html>
"""

EXPECTED_A_LIGHT = {
    "title": "A Light in the Attic",
    "price": Decimal("51.77"),
    "in_stock": 22,
    "rating": 3,
    "url": A_LIGHT_URL,
}

AI_REPLY = json.dumps(
    {"title": "A Light in the Attic", "price": "51.77", "in_stock": 22, "rating": 3}
)


class NetworkAttempt(AssertionError):
    """Raised when a test would reach the network. Names the address it tried."""

    def __init__(self, address: object) -> None:
        super().__init__(f"network access refused: the test tried to reach {address}")


class ScriptedModel:
    """Stands in for a model: answers from a fixed script and fails on any call the script does not cover."""

    def __init__(self, replies: list[str]) -> None:
        self._replies: deque[str] = deque(replies)
        self.prompts: list[str] = []

    def complete(self, prompt: str) -> str:
        self.prompts.append(prompt)
        if not self._replies:
            raise AssertionError("the test called a model that has no scripted reply left")
        return self._replies.popleft()


def html_response(url: str, body: str) -> HtmlResponse:
    return HtmlResponse(url=url, body=body.encode("utf-8"), encoding="utf-8")


def _pod(url: str, title: str) -> str:
    return (
        f'<article class="product_pod"><h3><a href="{url}" title="{title}">{title}</a></h3>'
        '<p class="price_color">£0.00</p></article>'
    )


def _category_page(pods: list[str], next_href: str | None = None) -> str:
    pager = (
        f'<ul class="pager"><li class="next"><a href="{next_href}">next</a></li></ul>'
        if next_href
        else ""
    )
    return "<html><body><section>" + "".join(pods) + "</section>" + pager + "</body></html>"


def _book_page(title: str, price: str, stock: int, rating: str) -> str:
    return f"""<!DOCTYPE html>
<html lang="en-us">
<head><title>{title} | Books to Scrape</title></head>
<body>
<article class="product_page"><div class="row"><div class="col-sm-6 product_main">
<h1>{title}</h1>
<p class="price_color">{price}</p>
<p class="instock availability"><i class="icon-ok"></i> In stock ({stock} available)</p>
<p class="star-rating {rating}"><i class="icon-star"></i></p>
</div></div></article>
</body>
</html>"""


HOME_PAGE = f"""<html><body>
<div class="side_categories"><ul class="nav nav-list"><li><a href="{BASE}/catalogue/category/books_1/index.html">Books</a>
<ul>
<li><a href="{POETRY_URL}">
    Poetry
</a></li>
<li><a href="{TRAVEL_URL}">Travel</a></li>
</ul></li></ul></div>
</body></html>"""

POETRY_PAGE_1 = _category_page(
    [_pod(A_LIGHT_URL, "A Light in the Attic"), _pod(TIPPING_URL, "Tipping the Velvet")],
    next_href="page-2.html",
)
POETRY_PAGE_2 = _category_page([_pod(SOUMISSION_URL, "Soumission")])
TRAVEL_PAGE = _category_page([_pod(HIMALAYAS_URL, "It's Only the Himalayas")])

# The recorded responses of the crawl, keyed by the URL each was served for.
RECORDED: dict[str, str] = {
    HOME_URL: HOME_PAGE,
    POETRY_URL: POETRY_PAGE_1,
    POETRY_PAGE_2_URL: POETRY_PAGE_2,
    TRAVEL_URL: TRAVEL_PAGE,
    A_LIGHT_URL: SAVED_BOOK_PAGE,
    TIPPING_URL: _book_page("Tipping the Velvet", "£53.74", 20, "One"),
    SOUMISSION_URL: _book_page("Soumission", "£50.10", 20, "One"),
    HIMALAYAS_URL: _book_page("It's Only the Himalayas", "£45.17", 19, "Two"),
}


def replay_crawl(spider: BooksSpider, recorded: dict[str, str]) -> list[dict[str, Any]]:
    """Runs the spider's callbacks over recorded responses, following each yielded Request as the engine
    would. A Request with no recorded response is a network fetch, so it is refused."""
    items: list[dict[str, Any]] = []
    pending: deque[scrapy.Request] = deque(
        scrapy.Request(url, callback=spider.parse) for url in spider.start_urls
    )
    while pending:
        request = pending.popleft()
        if request.url not in recorded:
            raise NetworkAttempt(f"{request.url}, which has no recorded response")
        callback = request.callback
        assert callback is not None
        response = html_response(request.url, recorded[request.url])
        for result in callback(response, **request.cb_kwargs):
            if isinstance(result, scrapy.Request):
                pending.append(result)
            else:
                items.append(result)
    return items


@pytest.fixture(autouse=True)
def refuse_network(monkeypatch: pytest.MonkeyPatch) -> None:
    """Refuses every socket entry point, so a test that reaches the network fails and names the address."""

    def refuse_connect(sock: socket.socket, address: object) -> NoReturn:
        raise NetworkAttempt(address)

    def refuse_connect_ex(sock: socket.socket, address: object) -> NoReturn:
        raise NetworkAttempt(address)

    def refuse_create_connection(address: object, *args: object, **kwargs: object) -> NoReturn:
        raise NetworkAttempt(address)

    def refuse_getaddrinfo(host: object, port: object, *args: object, **kwargs: object) -> NoReturn:
        raise NetworkAttempt(f"{host}:{port}")

    def refuse_gethostbyname(host: object) -> NoReturn:
        raise NetworkAttempt(host)

    monkeypatch.setattr(socket.socket, "connect", refuse_connect)
    monkeypatch.setattr(socket.socket, "connect_ex", refuse_connect_ex)
    monkeypatch.setattr(socket, "create_connection", refuse_create_connection)
    monkeypatch.setattr(socket, "getaddrinfo", refuse_getaddrinfo)
    monkeypatch.setattr(socket, "gethostbyname", refuse_gethostbyname)


def test_network_attempt_fails_and_names_the_address() -> None:
    with pytest.raises(NetworkAttempt, match="books.toscrape.com"):
        socket.create_connection(("books.toscrape.com", 443), timeout=1)


@pytest.mark.parametrize("category", ["Poetry", "Travel"])
def test_parse_book_yields_the_book_with_the_category_passed_in(category: str) -> None:
    spider = BooksSpider()
    items = list(
        spider.parse_book(html_response(A_LIGHT_URL, SAVED_BOOK_PAGE), category=category)
    )
    assert items == [{**EXPECTED_A_LIGHT, "category": category}]
    assert spider.skips == []


def test_whole_crawl_from_recorded_responses_holds_the_expected_books() -> None:
    spider = BooksSpider()
    items = replay_crawl(spider, RECORDED)
    assert len(items) == 4
    assert {item["url"]: item["category"] for item in items} == {
        A_LIGHT_URL: "Poetry",
        TIPPING_URL: "Poetry",
        SOUMISSION_URL: "Poetry",
        HIMALAYAS_URL: "Travel",
    }
    assert spider.failures == []
    assert spider.skips == []


def test_ai_extraction_from_a_scripted_reply_agrees_with_the_handler() -> None:
    model = ScriptedModel([AI_REPLY])
    extracted = json.loads(model.complete(f"Extract the book fields from this page.\n{SAVED_BOOK_PAGE}"))
    handled = next(
        iter(BooksSpider().parse_book(html_response(A_LIGHT_URL, SAVED_BOOK_PAGE), category="Poetry"))
    )
    assert extracted == {
        "title": handled["title"],
        "price": str(handled["price"]),
        "in_stock": handled["in_stock"],
        "rating": handled["rating"],
    }
    assert len(model.prompts) == 1
