"""A program's pages and their responses, on the real engine.

The engine is Collie's executable of the same commit, three ways: async on
asyncio, async on Trio and through the sync `Collie`. It launches the
browser of the test's own Browser cache and reaches the test's Proving
ground (`with_browser`). `goto`, `reload`, `go_back`, `go_forward` and
`request` answer a `Response`, whose body reads, decodes and fails as ADRs
0085, 0118 and 0119 say. Rust's `tests/it/sdk/page.rs` and TypeScript's
`page.test.ts` are its twins.
"""

from collections.abc import Mapping
from typing import cast
from urllib.parse import urljoin

import anyio.to_thread
import pytest
from collie_calls import failure, open_collie, response_on
from goodfetch_collie import (
    BodyUnavailableError,
    ResponseNotJSONError,
    ResponseTruncatedError,
    SettingInvalidError,
    TooManyRedirectsError,
)
from goodfetch_test_support import ClientCaseRecord, ProvingGround, Way, three_ways

FETCH_EDGE = "fetch-edge.test"
"""The Proving ground's host of edge cases of the HTTP layer."""


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests."""
    return 3394


@pytest.fixture
def with_browser() -> bool:
    """The engine opens pages on the Browser cache laid out in the test's home."""
    return True


async def start_url(proving_ground: ProvingGround, task: str) -> str:
    """The URL the Proving ground's task `task` starts at, read off the event loop."""
    return await anyio.to_thread.run_sync(proving_ground.start_url, task)


@pytest.mark.covers("story:11-84", "story:11-85", "operation:response.body")
@three_ways
async def test_a_navigations_response_gives_its_head_and_its_documents_text(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """Its text decodes the document as the browser did; a 200 is `ok`."""
    product = await start_url(proving_ground, "catalog/2")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    navigated = await page.goto(product)
    assert navigated is not None, "a document has a response"
    response = response_on(way, navigated)
    text = await response.text()
    body = await response.body()
    await run.finish()
    await collie.close()

    assert (response.status, response.url, response.ok) == (200, product, True)
    assert response.headers.get("content-type", "").startswith("text/html")
    assert response.encoding == "UTF-8"
    assert '<h1 class="name">' in text
    assert text == body.decode("utf-8", "replace")


@pytest.mark.covers("story:11-84")
@three_ways
async def test_goto_answers_the_response_of_a_404(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """A 404 is a value, the document's response, which is not `ok`."""
    start = await start_url(proving_ground, "statuses/1")
    missing = urljoin(start, "/errors/404")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    navigated = await page.goto(missing)
    await run.finish()
    await collie.close()

    assert navigated is not None, "a 404 is a response"
    assert (navigated.status, navigated.url, navigated.ok) == (404, missing, False)


@pytest.mark.covers("story:11-77")
@three_ways
async def test_a_runs_second_page_opens_in_its_session_and_another_sessions_apart(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """The second page reads what the first stored; another Session's reads none."""
    form = await start_url(proving_ground, "actionability/20")
    collie = await open_collie(way, env)
    run = await collie.start()
    first = await run.new_page()
    second = await run.new_page()
    session = await run.new_session()
    other = await session.new_page()
    for page in (first, second, other):
        await page.goto(form)

    await first.evaluate("localStorage.setItem('kept', 'by the first page')")
    shared = await second.evaluate("localStorage.getItem('kept')")
    apart = await other.evaluate("localStorage.getItem('kept')")
    await run.finish()
    await collie.close()

    assert (shared, apart) == ("by the first page", None)


@pytest.mark.covers(
    "story:11-85",
    "operation:page.request",
    "operation:response.body",
    "error:response_not_json",
)
@three_ways
async def test_text_honours_a_legacy_charset_and_json_ignores_a_bom(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """`Shift_JIS` from the `Content-Type`; JSON after a BOM; no JSON names the URL."""
    shift_jis = proving_ground.url(FETCH_EDGE, "/text/shift-jis")
    bom = proving_ground.url(FETCH_EDGE, "/json/bom")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    text_response = response_on(way, await page.request(shift_jis))
    japanese = await text_response.text()
    not_json = await failure(text_response.json())
    parsed = await response_on(way, await page.request(bom)).json()
    await run.finish()
    await collie.close()

    assert (japanese, parsed) == ("日本語", {"bom": True})
    assert isinstance(not_json, ResponseNotJSONError)
    assert not_json.url == shift_jis


@pytest.mark.covers(
    "story:11-85", "operation:response.body", "error:response_truncated"
)
@three_ways
async def test_a_cut_body_raises_response_truncated_from_body_text_and_json(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """A body cut mid-transfer is never read as whole."""
    cut = proving_ground.url(FETCH_EDGE, "/cut/content-length")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    response = response_on(way, await page.request(cut))
    failures = [
        await failure(response.body()),
        await failure(response.text()),
        await failure(response.json()),
    ]
    await run.finish()
    await collie.close()

    assert [type(failed) for failed in failures] == [ResponseTruncatedError] * 3


@pytest.mark.covers("story:11-85", "operation:response.body", "error:body_unavailable")
@three_ways
async def test_a_dropped_body_raises_body_unavailable_naming_the_waiter(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """The browser drops a document's body once the page navigates away."""
    first = await start_url(proving_ground, "catalog/2")
    then = await start_url(proving_ground, "actionability/20")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    left = await page.goto(first)
    assert left is not None, "a document has a response"
    await page.goto(then)
    dropped = await failure(response_on(way, left).text())
    await run.finish()
    await collie.close()

    assert isinstance(dropped, BodyUnavailableError)
    assert "wait_for_request_finished" in (dropped.fix or ""), dropped


@pytest.mark.covers(
    "story:11-84", "operation:page.go_back", "operation:page.go_forward"
)
@three_ways
async def test_back_and_forward_answer_the_documents_response_and_none_past_the_history(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """A cross-document entry loads and answers its document's response."""
    first = await start_url(proving_ground, "catalog/2")
    then = await start_url(proving_ground, "actionability/20")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()
    await page.goto(first)
    await page.goto(then)

    back = await page.go_back()
    forward = await page.go_forward()
    past = await page.go_forward()
    await run.finish()
    await collie.close()

    assert back is not None, "the first document's entry"
    assert forward is not None, "the second document's entry"
    assert (back.url, forward.url, past) == (first, then, None)


@pytest.mark.covers("story:11-87", "operation:page.request")
@pytest.mark.client_case(
    id="page-request-sends-the-sessions-cookie-through-its-proxy", mode="attached"
)
@three_ways
async def test_page_request_sends_the_sessions_cookie_through_its_proxy(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """On a Run whose `proxy.server` is the open proxy, as its record shows."""
    open_proxy = proving_ground.fixtures.proxies["open"]
    storage = proving_ground.url("scope.test", "/storage?mark=page-request")
    about = proving_ground.url("scope.test", "/about?by=page-request")
    collie = await open_collie(way, env)
    run = await collie.start(settings={"proxy": {"server": f"http://{open_proxy}"}})
    page = await run.new_page()
    await page.goto(storage)

    response = await page.request(about)
    await run.finish()
    await collie.close()
    observations = await anyio.to_thread.run_sync(proving_ground.observations)

    requests = cast("list[dict[str, object]]", observations["requests"])
    relays = cast("list[dict[str, object]]", observations.get("relays", []))
    with client_case.check("`page.request` from the Session answers 200"):
        assert response.status == 200
    with client_case.check("it sends the cookie a page of its Session got"):
        assert [
            request["cookies"]
            for request in requests
            if (request["host"], request["target"])
            == ("scope.test", "/about?by=page-request")
        ] == [[["mark", "page-request"]]]
    with client_case.check("it leaves through the Run's proxy"):
        assert any(
            relay["proxy"] == "open"
            and relay["outcome"] == "relayed"
            and str(relay["target"]).endswith("/about?by=page-request")
            for relay in relays
        ), relays


@pytest.mark.covers(
    "story:11-87",
    "operation:page.request",
    "error:too_many_redirects",
    "error:setting_invalid",
)
@pytest.mark.client_case(
    id="page-request-follows-its-max-redirects-and-refuses-a-count-above-19",
    mode="attached",
)
@three_ways
async def test_page_request_follows_its_max_redirects_and_refuses_a_count_above_19(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """On the Proving ground's chain of 5 redirects (ADR 0118)."""
    chain = proving_ground.url(FETCH_EDGE, "/redirect/hops/5")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    refused = await failure(page.request(chain, max_redirects=20))
    unfollowed = await page.request(chain, max_redirects=0)
    within = await page.request(chain, max_redirects=5)
    past = await failure(page.request(chain, max_redirects=3))
    await run.finish()
    await collie.close()

    with client_case.check("`max_redirects` 20 is refused, naming the option"):
        assert isinstance(refused, SettingInvalidError), refused
        assert refused.details.get("key") == "max_redirects", refused.details
    with client_case.check("`max_redirects` 0 answers the 3xx itself"):
        assert unfollowed.status == 302
        assert unfollowed.url.endswith("/redirect/hops/5"), unfollowed.url
        assert unfollowed.headers.get("location") == "/redirect/hops/4"
    with client_case.check("a chain within its count answers its last response"):
        assert within.status == 200
        assert within.url.endswith("/redirect/hops/0"), within.url
    with client_case.check(
        "past its count, `too_many_redirects` names the chain, its hops and the option"
    ):
        assert isinstance(past, TooManyRedirectsError), past
        hops = cast("list[str]", past.details["hops"])
        assert (past.url, len(hops), past.details["limit"]) == (
            chain,
            3,
            {"kind": "option", "name": "max_redirects", "count": 3},
        )
