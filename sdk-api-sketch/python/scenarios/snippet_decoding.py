"""Snippet for Decoding a response the same way on every tier (#550, ADR 0119)."""

import logging
from urllib.parse import quote

from pydantic import BaseModel

from goodfetch_foxhound import AsyncCrawl, AsyncCrawlContext, AsyncFoxhound, CrawlRequestSkipped, seed

log = logging.getLogger(__name__)


class Notice(BaseModel):
    title: str
    url: str


class File(BaseModel):
    url: str
    content_type: str
    size: int


crawl = AsyncCrawl()
notices = crawl.dataset("notices", schema=Notice)
files = crawl.dataset("files", schema=File)
# HTML pages, the built-in `crawl.content_types`. The site's legacy section declares no charset, so its Label forces
# one rather than escalating each page to the browser.
listing = crawl.label("listing", settings={"fetch": {"encoding": "shift_jis"}})
# The Label takes PDFs too: their bytes are in `ctx.response`, and their page holds no document.
attachment = crawl.label("attachment", settings={"crawl": {"content_types": ["text/html", "application/pdf"]}})


@crawl.handle(listing)
async def on_listing(ctx: AsyncCrawlContext[None]) -> None:
    await ctx.push(notices, Notice(title=await ctx.page.locator("h1").text_content(), url=ctx.page.url))
    await ctx.enqueue_links(ctx.page.locator("a.attachment"), attachment)


@crawl.handle(attachment)
async def on_attachment(ctx: AsyncCrawlContext[None]) -> None:
    content_type = ctx.response.headers.get("content-type", "")
    if content_type.startswith("application/pdf"):
        # `ctx.page.locator(…)` here would raise `ResponseNotDocumentError` at once, on every tier.
        body = await ctx.response.body()
        await ctx.run.key_value_store.set_bytes(f"pdf/{quote(ctx.page.url, safe='')}", body, "application/pdf")
        await ctx.push(files, File(url=ctx.page.url, content_type=content_type, size=len(body)))
        return
    # `text()` decodes with the document's own encoding, so it agrees with `page.content()`.
    log.debug("attachment page %s: %d characters", ctx.page.url, len(await ctx.response.text()))


async def decoding() -> None:
    async with AsyncFoxhound.connect() as foxhound:
        run = await foxhound.start_crawl(
            crawl,
            seeds=[seed("https://example.org/notices/", listing)],
            settings={"export": {"csv": {"bom": True}}},  # the CSV Export opens in a spreadsheet
        )
        async with run:
            async for outcome in run.outcomes(kinds=["skipped"]):
                # A ZIP or an image under `attachment`: skipped before its body is read.
                if isinstance(outcome, CrawlRequestSkipped) and outcome.reason == "content_type":
                    log.info("not collected %s: %s", outcome.request.url, outcome.message)
            await run.finish()
            csv = await run.export(notices, format="csv")  # UTF-8, with a BOM under `export.csv.bom`
            await csv.save_as("notices.csv")
