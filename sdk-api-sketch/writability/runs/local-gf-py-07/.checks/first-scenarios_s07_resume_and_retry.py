```python
from scenarios.s05_two_page_crawl import *

# Your implementation for resume and retry logic goes here

async def main() -> None:
    async with (
        AsyncFoxhound.connect() as foxhound,
        await foxhound.start_crawl(crawl, seeds=seeds, name="my_crawl") as run,
    ):
        report = await run.finish()
        exported = await run.export(books, format="json")
        await exported.save_as("books.json")
        print(f"exported {exported.records} books to books.json")

        async for outcome in run.outcomes(kinds=["failed", "skipped", "blocked"]):
            match outcome:
                case CrawlRequestFailed(request=request, error=error):
                    print(f"failed {request.url}: {error.code}: {error}")
                case CrawlRequestSkipped(
                    request=request, reason=reason, message=message
                ):
                    print(f"skipped {request.url}: {reason} {message or ''}")
                case CrawlRequestBlocked(request=request, block_cause=cause):
                    print(f"blocked {request.url}: {cause}")
                case _:
                    pass
        totals = report.requests
        print(
            f"{totals.succeeded} succeeded, {totals.failed} failed, {totals.skipped} skipped, {totals.blocked} blocked"
        )
        if report.nothing_done is not None:
            print(f"nothing done: {report.nothing_done.reason}")


if __name__ == "__main__":
    asyncio.run(main())
```