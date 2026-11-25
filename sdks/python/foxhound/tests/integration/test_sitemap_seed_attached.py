"""A crawl's sitemap seed on Foxhound's executable (ADRs 0093 and 0120; issue 1471).

A program starts a crawl from `sitemap(url, rules)` on an Attached engine
of Foxhound's executable of the same commit, beside a request seed, and
the engine reads the Proving ground's `sitemap-seed` family: each entry
is handled by the Label of the first rule whose pattern matches its URL.
The Rust SDK's `tests/it/sdk/seeds.rs` and TypeScript's
`sitemap-seed.test.ts` of Foxhound's package are its twins.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, seed, sitemap
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import AsyncCrawlContext
    from goodfetch_foxhound.generated.settings import FoxhoundSettings
    from goodfetch_test_support import ProvingGround

HOST = "sitemap-seed.test"
"""The `sitemap-seed` family's host."""


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


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests, Rust's."""
    return 903


@pytest.mark.covers("story:11-1019", "operation:crawl.start")
@pytest.mark.anyio
async def test_a_sitemap_seed_routes_each_entry_to_the_label_of_its_first_matching_rule(
    env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """A book matches `book`'s rule and the later `shelf`'s, and is a `book`.

    The index's two sitemaps list authors 1 and 2 and books 1 to 3 on the
    family's host; `/about` matches no rule and the partner's book is out
    of the crawl's scope, so neither is handled. The request seed, of the
    first book, is a duplicate of its sitemap entry.
    """
    url = proving_ground.url(HOST, "")
    crawl = AsyncCrawl()
    book = crawl.label("book")
    shelf = crawl.label("shelf")
    author = crawl.label("author")
    handled: list[tuple[str, str]] = []

    async def record(ctx: AsyncCrawlContext[None]) -> None:
        handled.append((ctx.request.label, ctx.request.url.removeprefix(url)))

    for label in (book, shelf, author):
        crawl.handle(label)(record)

    foxhound = await AsyncFoxhound.connect(env=env, settings=served(proving_ground))
    run = await foxhound.start_crawl(
        crawl,
        seeds=[
            seed(f"{url}/books/1", book),
            sitemap(
                f"{url}/sitemap-index.xml",
                [
                    ("**/books/*", book),
                    ("**/books/**", shelf),
                    ("**/authors/*", author),
                ],
            ),
        ],
    )
    report = await run.finish()
    await foxhound.close()

    assert report.lifecycle == "finished"
    assert sorted(handled) == [
        ("author", "/authors/1"),
        ("author", "/authors/2"),
        ("book", "/books/1"),
        ("book", "/books/2"),
        ("book", "/books/3"),
    ]
