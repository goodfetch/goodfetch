import asyncio

from goodfetch_collie import AsyncCollie
from playwright.async_api import Page, async_playwright
from pydantic import BaseModel


class Book(BaseModel):
    title: str
    price: str


class Books(BaseModel):
    books: list[Book]


async def extract_books(page: Page) -> list[Book]:
    """Borrows the program's own page for a model-driven extraction, then releases it.

    Borrowing never closes or navigates the browser; closing the borrowed page only releases it.
    """
    async with AsyncCollie.connect() as collie:
        async with await collie.start() as run:
            borrowed = await run.borrow(page)
            result = await borrowed.extract(
                "Extract the title and price of every book listed on this page.", Books
            )
            await borrowed.close()
            await run.finish()
            return result.output.books


async def main() -> None:
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://books.toscrape.com/")
        await page.get_by_role("link", name="Travel").click()
        try:
            for book in await extract_books(page):
                print(f"{book.title}: {book.price}")
        except Exception as error:
            print(f"AI extraction failed, carrying on: {error!r}")
        await page.locator("article.product_pod h3 a").first.click()
        print(await page.locator(".product_main h1").text_content())
        await browser.close()


asyncio.run(main())
