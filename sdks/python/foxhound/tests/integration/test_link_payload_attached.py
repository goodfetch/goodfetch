"""A handler's enqueues on the `links` family (ADRs 0093 and 0120; issue 1471).

On an Attached engine of Foxhound's executable of the same commit,
`enqueue_links`' Payload function reads each element the locator matches
on the Proving ground's `links` family, as `crawl_request.links` answers
it, and gives each link its own Payload, or `None` to leave it out; and a
relative `enqueue` resolves against the base URL of the document its
handler was given. TypeScript's `link-payload.test.ts` and
`enqueue-base.test.ts` of Foxhound's package are its twins, and
Foxhound's crate's `tests/it/enqueue_links.rs` the engine's side.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, seed
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import AsyncCrawlContext
    from goodfetch_foxhound.generated.models import (
        Enqueued,
        EnqueuedLinks,
        MatchedLink,
    )
    from goodfetch_foxhound.generated.settings import FoxhoundSettings
    from goodfetch_test_support import ProvingGround

HOST = "links.test"
"""The `links` family's host."""


def served(proving_ground: ProvingGround) -> FoxhoundSettings:
    """The Connection's settings: plain HTTP fetches, the Proving ground reached.

    Each of its hosts resolves to its address, reachable although private.
    """
    resolve = proving_ground.resolve
    return {
        "network": {"resolve": {host: [address] for host, address in resolve.items()}},
        "egress": {"private_ranges": list(resolve)},
        "fetch": {"tier": "http"},
    }


class Linked(BaseModel):
    """A linked page's Payload: the link's text, or its book's ISBN."""

    text: str | None = None
    isbn: str | None = None


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests, Rust's."""
    return 863


@pytest.mark.covers("story:11-1022", "operation:crawl_request.enqueue")
@pytest.mark.anyio
async def test_a_relative_enqueue_resolves_against_its_documents_base_url(
    env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """`book/1` resolves against the `<base href>` or the URL its shelf moved to.

    `links/1`'s shelf names `<base href="/library/shelves/">`, and
    `links/2`'s moved from `/old/shelf` to `/new/shelf/`; a wrong base
    reaches a `404`.
    """
    shelves = {n: proving_ground.url(f"{n}.{HOST}", "") for n in (1, 2)}
    crawl = AsyncCrawl()
    shelf = crawl.label("shelf")
    book = crawl.label("book")
    answered: list[Enqueued] = []
    handled: list[str] = []

    @crawl.handle(shelf)
    async def follow(ctx: AsyncCrawlContext[None]) -> None:
        answered.append(await ctx.enqueue("book/1", book))

    @crawl.handle(book)
    async def record(ctx: AsyncCrawlContext[None]) -> None:
        handled.append(ctx.request.url)

    foxhound = await AsyncFoxhound.connect(env=env, settings=served(proving_ground))
    run = await foxhound.start_crawl(
        crawl,
        seeds=[
            seed(f"{shelves[1]}/guide/", shelf),
            seed(f"{shelves[2]}/old/shelf", shelf),
        ],
    )
    report = await run.finish()
    await foxhound.close()

    books = [f"{shelves[1]}/library/shelves/book/1", f"{shelves[2]}/new/shelf/book/1"]
    assert report.lifecycle == "finished"
    assert sorted(answer.root.url for answer in answered) == books
    assert sorted(handled) == books


@pytest.mark.covers("story:11-1021", "operation:crawl_request.enqueue_links")
@pytest.mark.anyio
async def test_a_payload_function_gives_each_link_its_payload_or_leaves_it_out(
    env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """A book's link takes its ISBN, the link `2` is left out, any other its text.

    The function reads every matched element, an element without an `href`
    included, and the engine answers each one as its Frontier judged it.
    """
    url = proving_ground.url(HOST, "")
    crawl = AsyncCrawl()
    index = crawl.label("index")
    page = crawl.label("page", payload=Linked)
    read: list[str] = []
    answered: list[EnqueuedLinks] = []

    def linked(link: MatchedLink) -> Linked | None:
        read.append(link.text)
        isbn = link.attributes.get("data-isbn")
        if isbn is not None:
            return Linked(isbn=isbn)
        return None if link.href == "2" else Linked(text=link.text)

    @crawl.handle(index)
    async def follow(ctx: AsyncCrawlContext[None]) -> None:
        answered.append(
            await ctx.enqueue_links(ctx.page.locator("ul.links a"), page, linked)
        )

    @crawl.handle(page)
    async def ignore(ctx: AsyncCrawlContext[Linked]) -> None:
        del ctx

    foxhound = await AsyncFoxhound.connect(env=env, settings=served(proving_ground))
    run = await foxhound.start_crawl(crawl, seeds=[seed(f"{url}/answers/", index)])
    report = await run.finish()
    await foxhound.close()

    assert report.lifecycle == "finished"
    assert read == [
        "One",
        "One again",
        "No link",
        "Mail",
        "Elsewhere",
        "Broken",
        "Two",
        "Three",
    ]
    assert [link.model_dump(exclude_none=True) for link in answered[0].links] == [
        {"kind": "added", "id": "2", "url": f"{url}/answers/1"},
        {"kind": "not_added", "reason": "duplicate", "url": "/answers/1#top"},
        {"kind": "no_link"},
        {
            "kind": "not_added",
            "reason": "unsupported_scheme",
            "url": "mailto:shop@links.test",
        },
        {
            "kind": "not_added",
            "reason": "out_of_scope",
            "url": "http://elsewhere.test/",
        },
        {"kind": "not_added", "reason": "invalid_url", "url": "http://[::1/"},
        {"kind": "skipped_by_payload", "url": "2"},
        {"kind": "added", "id": "3", "url": f"{url}/answers/3"},
    ]
