import asyncio
import sys

from goodfetch_collie import AsyncCollie
from playwright.async_api import Page, async_playwright
from pydantic import BaseModel


class Book(BaseModel):
    title: str
    price: str


class BookList(BaseModel):
    books: list[Book]


EXTRACT_INSTRUCTION = (
    "Extract every book listed on this page, in the order shown. "
    "For each book give its title and its price, with the currency symbol."
)


async def extract_books(page: Page) -> None:
    """Prints the title and price of every book listed on the page, extracted with the model `ai.model` names.

    Collie borrows the page the program already drives, so nothing is opened, navigated or closed on its behalf: closing
    the connection only releases the borrowed page and never closes the browser. A failure is reported and not raised,
    so the program carries on with its page and closes its browser as before.
    """
    try:
        async with AsyncCollie.connect() as collie:
            async with await collie.start() as run:
                borrowed = await run.borrow(page)
                result = await borrowed.extract(EXTRACT_INSTRUCTION, BookList)
                for book in result.output.books:
                    print(f"{book.title}: {book.price}")
    except Exception as error:
        print(f"Extracting the books failed: {error!r}", file=sys.stderr)


async def main() -> None:
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://books.toscrape.com/")
        await page.get_by_role("link", name="Travel").click()
        await extract_books(page)
        await page.locator("article.product_pod h3 a").first.click()
        print(await page.locator(".product_main h1").text_content())
        await browser.close()


asyncio.run(main())
