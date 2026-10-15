import asyncio

from playwright.async_api import async_playwright


async def main() -> None:
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://books.toscrape.com/")
        await page.get_by_role("link", name="Travel").click()
        await page.locator("article.product_pod h3 a").first.click()
        print(await page.locator(".product_main h1").text_content())
        await browser.close()


asyncio.run(main())
