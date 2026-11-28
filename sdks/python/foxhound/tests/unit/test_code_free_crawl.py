"""A code-free crawl (ADR 0093) on the Scripted engine (ADR 0069).

`start_crawl` given a reviewed `CrawlDeclaration` sends it as `crawl.start`'s
`declaration`, with seeds naming their Label by name and `detached`, and the
program reads a declared Dataset by its name through the raw `call`
(ADR 0099). TypeScript's `code-free.test.ts` of `@goodfetch/foxhound` is its
twin.
"""

from __future__ import annotations

import pytest
from goodfetch_foxhound import CrawlDeclaration, seed
from goodfetch_foxhound.testing import async_connect_over, connect_over
from goodfetch_test_support import Finished, play_script

SCRIPT = "sdk-python/crawl/code-free.json"
"""The code-free crawl started detached, its Dataset read by name."""
SEED = "https://shop.test/catalog/1"
"""The crawl's one seed, a listing page."""
DECLARATION = CrawlDeclaration.model_validate(
    {
        "labels": [
            {
                "name": "listing",
                "follow": [{"links": "a.next", "label": "listing", "limit": 10}],
                "extract": [
                    {
                        "dataset": "products",
                        "fields": {
                            "id": {
                                "locator": {
                                    "kind": "css",
                                    "selector": "article.product .id",
                                }
                            }
                        },
                    }
                ],
            }
        ],
        "datasets": [{"name": "products", "key": ["id"]}],
    }
)
"""Listings that follow their Next link and extract each product's ID."""
PAGE = {
    "records": [
        {
            "number": 1,
            "value": {"id": "p-1"},
            "request_id": "1",
            "url": SEED,
            "committed_at": "2026-01-01T00:00:01Z",
            "provenance": {"request_id": "1"},
        }
    ],
    "through": 1,
}
"""The page of `products` the script answers."""


@pytest.mark.covers("story:11-1069")
@pytest.mark.anyio
async def test_starts_detached_from_a_declaration_and_reads_its_dataset_by_name() -> (
    None
):
    """The start carries the declaration, and `call` reads the Dataset by name."""
    played = play_script(SCRIPT)
    foxhound = await async_connect_over(played.transport)

    run = await foxhound.start_crawl(
        DECLARATION, seeds=[seed(SEED, "listing")], run_id="catalog", detached=True
    )
    page = await foxhound.call(
        "dataset.page", {"run_id": "catalog", "dataset": "products"}
    )
    await run.close()
    await foxhound.close()

    assert (run.id, page) == ("catalog", PAGE)
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1069")
def test_a_sync_connection_starts_the_same_code_free_crawl() -> None:
    """The sync twin."""
    played = play_script(SCRIPT)

    with connect_over(played.transport) as foxhound:
        run = foxhound.start_crawl(
            DECLARATION, seeds=[seed(SEED, "listing")], run_id="catalog", detached=True
        )
        page = foxhound.call(
            "dataset.page", {"run_id": "catalog", "dataset": "products"}
        )
        run.close()

    assert (run.id, page) == ("catalog", PAGE)
    assert played.done() == Finished()
