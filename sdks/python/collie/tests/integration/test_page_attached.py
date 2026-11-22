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
from urllib.parse import urljoin

import anyio.to_thread
import pytest
from collie_calls import failure, open_collie, response_on
from goodfetch_collie import (
    BodyUnavailableError,
    ResponseNotJSONError,
    ResponseTruncatedError,
)
from goodfetch_test_support import ProvingGround, Way, three_ways

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
