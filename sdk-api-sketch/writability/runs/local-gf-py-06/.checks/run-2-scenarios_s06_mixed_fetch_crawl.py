from goodfetch_collie_client import AsyncPage, AsyncResponse, AsyncLocator, AsyncCrawlContext, AsyncCrawl, Label, Dataset, CrawlRequest
from goodfetch_collie_client._page import AsyncLocator, AsyncResponse, AsyncPage
from goodfetch_collie_client._lifecycle import AsyncCrawlContext, AsyncCrawl
from goodfetch_collie_client._models import Label, Dataset

async def handle_quote_page(ctx: AsyncCrawlContext[None]):
    pass

async def handle_scroll_page(ctx: AsyncCrawlContext[None]):
    pass

async def handle_js_page(ctx: AsyncCrawlContext[None]):
    pass

crawl = AsyncCrawl()
crawl.label(name='quote_page', settings={}).handle(handle_quote_page)
crawl.label(name='scroll_page', settings={}).handle(handle_scroll_page)
crawl.label(name='js_page', settings={}).handle(handle_js_page)
