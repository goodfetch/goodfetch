import asyncio

from playwright.async_api import async_playwright

async def main() -> None:
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://books.toscrape.com/")
        await page.get_by_role("link", name="Travel").click()

        # New step: extract title and price of every book listed
        books = await page.locator("article.product_pod")
        for book in await books.all():
            title = await book.locator("h3 a").text_content()
            price = await book.locator(".price_color").text_content()
            print(f'Title: {title}, Price: {price}')

        await page.locator("article.product_pod h3 a").first.click()
        print(await page.locator(".product_main h1").text_content())
        await browser.close()


asyncio.run(main())