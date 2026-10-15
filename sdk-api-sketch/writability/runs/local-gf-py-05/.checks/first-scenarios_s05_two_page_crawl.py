from goodfetch_collie_client import AsyncPage, AsyncCrawl, AsyncCrawlContext
from goodfetch_foxhound import AsyncCrawlRun
from pydantic import BaseModel
import re
import json


class BookItem(BaseModel):
    title: str
    price: float
    in_stock: int
    star_rating: int
    page_address: str
    category_name: str


class CrawlData(BaseModel):
    succeeded: int = 0
    failed: int = 0
    skipped: int = 0
    failures: dict[str, str] = {}


crawl = AsyncCrawl()


@crawl.label(name='category_page', payload=BookItem)
async def category_page_handler(ctx: AsyncCrawlContext[BookItem]) -> None:
    try:
        page = ctx.page
        await page.goto(ctx.request.url)
        category_name = ctx.request.payload.category_name
        book_links = await page.get_by_role('link', name=re.compile(r'.*book.*')).all()
        for link in book_links:
            book_url = await link.get_attribute('href')
            await ctx.enqueue(url=book_url, label=book_page_label, payload={'category_name': category_name})
        next_page = await page.get_by_role('link', name='next').first
        await next_page.click()
    except Exception as e:
        ctx.run.failures[ctx.request.url] = str(e)
        ctx.skip(str(e))


@crawl.label(name='book_page', payload=BookItem)
async def book_page_handler(ctx: AsyncCrawlContext[BookItem]) -> None:
    try:
        page = ctx.page
        await page.goto(ctx.request.url)
        title = await page.get_by_role('heading', name='Book').first.text()
        price = await page.get_by_text('Price (incl. tax):').first.text()
        price = float(price.replace('£', '').replace(',', ''))
        in_stock = await page.get_by_text('In stock (', exact=False).first.text()
        in_stock = int(re.findall(r'\d+', in_stock)[0])
        star_rating = await page.get_by_css('p.star-rating').first.get_attribute('class')
        star_rating = len(re.findall(r'star', star_rating))
        category_name = ctx.request.payload.category_name
        book_item = BookItem(
            title=title,
            price=price,
            in_stock=in_stock,
            star_rating=star_rating,
            page_address=ctx.request.url,
            category_name=category_name
        )
        await ctx.push(dataset=book_dataset, record=book_item)
    except Exception as e:
        ctx.run.failures[ctx.request.url] = str(e)
        ctx.skip(str(e))


book_dataset = crawl.dataset(name='books', schema=BookItem)
book_page_label = crawl.label(name='book_page', payload=BookItem)
category_page_label = crawl.label(name='category_page', payload=BookItem)


async def main() -> None:
    run = await AsyncCrawlRun.start(crawl=crawl, seeds=[
        {'url': 'https://books.toscrape.com/', 'label': category_page_label, 'payload': {'category_name': 'Books'}},
    ])
    await run.wait_done()
    crawl_data = CrawlData(
        succeeded=len(run.succeeded),
        failed=len(run.failed),
        skipped=len(run.skipped),
        failures=run.failures
    )
    with open('crawl_data.json', 'w') as f:
        json.dump(crawl_data.dict(), f, indent=4)


if __name__ == '__main__':
    import asyncio
    asyncio.run(main())