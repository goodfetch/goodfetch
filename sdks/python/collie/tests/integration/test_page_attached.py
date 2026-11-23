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
from collie_calls import failure, locator_on, open_collie, response_on
from goodfetch_collie import (
    BodyUnavailableError,
    ClosedError,
    ResponseNotJSONError,
    ResponseTruncatedError,
    SettingInvalidError,
    TooManyRedirectsError,
)
from goodfetch_collie_client.generated.models import FormatsRequest, PageField
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


@pytest.mark.covers("story:11-77")
@pytest.mark.client_case(id="page-form-program", mode="attached")
@three_ways
async def test_the_form_program_fills_and_sends_the_form_and_reads_its_markdown(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """A field filled, a box clicked, Enter pressed, the end state's Markdown read."""
    form = await start_url(proving_ground, "actionability/20")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    navigated = await page.goto(form)
    await locator_on(way, page, "#name").fill("Goodfetch")
    await locator_on(way, page, "#consent").click()
    await locator_on(way, page, "#submit").press("Enter")
    await locator_on(way, page, "#done:not(:empty)").wait_for()
    formats = await page.formats(
        FormatsRequest.model_validate({"markdown": {"content": "full"}})
    )
    await page.close()
    report = await run.finish()
    await collie.close()

    with client_case.check("goto answers the form page's document"):
        assert navigated is not None
        assert navigated.status == 200
    with client_case.check("the page reached its end state: the form was sent"):
        assert "done" in (formats.markdown or "")
    with client_case.check("the Run finished"):
        assert report.lifecycle == "finished"


@pytest.mark.covers("story:11-84")
@pytest.mark.client_case(id="goto-404-returns-the-response", mode="attached")
@three_ways
async def test_goto_answers_the_response_of_a_404(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
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

    with client_case.check("a 404 is a value: the document's Response"):
        assert navigated is not None, "a 404 is a response"
        assert (navigated.status, navigated.url, navigated.ok) == (
            404,
            missing,
            False,
        )


@pytest.mark.covers("story:11-77")
@pytest.mark.client_case(id="second-new-page-in-the-default-session", mode="attached")
@three_ways
async def test_a_runs_second_page_opens_in_its_session_and_another_sessions_apart(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
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

    with client_case.check("the second page reads what the first stored: one Session"):
        assert shared == "by the first page"
    with client_case.check("a page of another Session reads nothing of it"):
        assert apart is None


PRODUCT_FIELDS = {
    "id": PageField.model_validate({"locator": {"kind": "css", "selector": "span.id"}}),
    "name": PageField.model_validate(
        {"locator": {"kind": "css", "selector": "h1.name"}}
    ),
    "price": PageField.model_validate(
        {
            "locator": {"kind": "css", "selector": "span.price"},
            "attribute": "data-price",
            "transform": {"kind": "number", "locale": "en"},
        }
    ),
    "missing": PageField.model_validate(
        {"locator": {"kind": "css", "selector": ".no-such-element"}}
    ),
}
"""The fields of the Proving ground's product page, and one no element answers."""


@pytest.mark.covers("story:11-77", "operation:page.read")
@pytest.mark.client_case(id="page-read-fields", mode="attached")
@three_ways
async def test_reads_a_product_pages_fields_in_one_call(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """The oracle verifies the Record; a field with no element is `None`, with why."""
    product = await start_url(proving_ground, "catalog/2")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()
    await page.goto(product)

    read = await page.read(PRODUCT_FIELDS)
    await run.finish()
    await collie.close()
    record = {name: value for name, value in read.values.items() if name != "missing"}
    answer = await anyio.to_thread.run_sync(proving_ground.judge, "catalog/2", [record])
    client_case.oracle_answer(answer)

    with client_case.check("the oracle verifies the Record the fields answer"):
        assert answer.outcome == "verified"
    with client_case.check("the number field answers a number"):
        assert isinstance(record["price"], float | int)
    with client_case.check("a field with no element is None, with its reason"):
        assert read.values["missing"] is None
        assert [(unread.field, unread.reason) for unread in read.unread or []] == [
            ("/missing", "no_element")
        ]


@pytest.mark.covers("story:11-77", "error:closed")
@pytest.mark.client_case(id="closed-page-raises-closed", mode="attached")
@three_ways
async def test_a_closed_page_and_a_locator_built_on_it_raise_closed(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """`goto` on the closed page and a click of its Locator raise `ClosedError`."""
    form = await start_url(proving_ground, "actionability/20")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()
    await page.goto(form)

    button = locator_on(way, page, "#submit")
    await page.close()
    navigation = await failure(page.goto(form))
    click = await failure(button.click())
    await run.finish()
    await collie.close()

    with client_case.check("goto on the closed page fails with closed"):
        assert isinstance(navigation, ClosedError)
        assert (navigation.code, navigation.group) == ("closed", "program")
    with client_case.check("a locator of the closed page fails with closed"):
        assert isinstance(click, ClosedError)
        assert (click.code, click.group) == ("closed", "program")


@pytest.mark.covers("story:11-84", "operation:page.goto")
@pytest.mark.client_case(
    id="goto-skips-a-redirects-retry-after-and-answers-none-for-a-file",
    mode="attached",
)
@three_ways
async def test_goto_skips_a_redirects_retry_after_and_answers_none_for_a_file(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """A program's own `goto` is outside Politeness, and a file is a download.

    The redirect's `Retry-After` asks for an hour, far past
    `timeout.navigation`, so its target answering is the proof, with no
    measured time (ADRs 0086, 0118). The PDF answers no response and the
    page keeps its document, which a reload loads again (ADR 0119).
    """
    redirect = proving_ground.url(FETCH_EDGE, "/retry-after/302/seconds/long")
    target = proving_ground.url(FETCH_EDGE, "/target")
    document = await start_url(proving_ground, "catalog/2")
    file = urljoin(await start_url(proving_ground, "files/2"), "/files/report.pdf")
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    followed = await page.goto(redirect)
    await page.goto(document)
    downloaded = await page.goto(file)
    reloaded = await page.reload()
    await run.finish()
    await collie.close()

    with client_case.check(
        "a 302 whose Retry-After asks for an hour is followed at once"
    ):
        assert followed is not None
        assert (followed.status, followed.url) == (200, target)
    with client_case.check("a goto to a file the browser downloads answers None"):
        assert downloaded is None
    with client_case.check("the page keeps its document, which a reload loads again"):
        assert reloaded is not None
        assert (reloaded.status, reloaded.url) == (200, document)


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
