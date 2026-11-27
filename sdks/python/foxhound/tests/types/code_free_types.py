"""A code-free crawl's start (ADRs 0093 and 0102), checked by pyright and mypy.

`start_crawl` takes `detached` only for a code-free crawl, a reviewed
`CrawlDeclaration` or a `CrawlExtraction`, since a crawl the program serves
waits on its handlers. mypy's `--strict` reports an ignore that no longer
silences an error. TypeScript's `code-free.test-d.ts` of Foxhound's package
is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, assert_type

from goodfetch_foxhound import AsyncCrawlRun, CrawlRun, seed

if TYPE_CHECKING:
    from goodfetch_foxhound import (
        AsyncCrawl,
        AsyncFoxhound,
        Crawl,
        CrawlDeclaration,
        CrawlExtraction,
        Foxhound,
    )


async def detaches_only_a_code_free_crawl(
    foxhound: AsyncFoxhound,
    declaration: CrawlDeclaration,
    extraction: CrawlExtraction,
    crawl: AsyncCrawl,
) -> None:
    """A declaration and an extraction detach; a served crawl takes no `detached`."""
    seeds = [seed("https://shop.test/", "page")]
    assert_type(
        await foxhound.start_crawl(declaration, seeds=seeds, detached=True),
        AsyncCrawlRun,
    )
    assert_type(
        await foxhound.start_crawl(extraction, seeds=seeds, detached=True),
        AsyncCrawlRun,
    )
    await foxhound.start_crawl(crawl, seeds=seeds, detached=True)  # type: ignore[call-overload]


def detaches_only_a_code_free_crawl_in_sync_code(
    foxhound: Foxhound, declaration: CrawlDeclaration, crawl: Crawl
) -> None:
    """The sync API takes `detached` for a code-free crawl only, as the async one."""
    seeds = [seed("https://shop.test/", "page")]
    assert_type(foxhound.start_crawl(declaration, seeds=seeds, detached=True), CrawlRun)
    foxhound.start_crawl(crawl, seeds=seeds, detached=True)  # type: ignore[call-overload]
