from goodfetch_collie_client import AsyncPage, AsyncRun, RunSettings, PageSettings, PlaywrightChromium

async def find_cheapest_five_star_book() -> None:
    run_settings = RunSettings()
    page_settings = PageSettings()
    async with PlaywrightChromium() as browser:
        async with browser.new_page() as page:
            await page.goto('https://books.toscrape.com/')
            # Add code to navigate to Travel category and find the cheapest five-star book
            # This is a placeholder for the actual implementation
            await page.wait_for_selector('h3 a')
            titles = await page.query_selector_all('h3 a')
            prices = await page.query_selector_all('.price_color')
            for title, price in zip(titles, prices):
                title_text = await title.text_content()
                price_text = await price.text_content()
                print(f'Title: {title_text}, Price: {price_text}')

# The actual implementation should navigate through pages, check ratings, and compare prices
if __name__ == '__main__':
    import asyncio
    asyncio.run(find_cheapest_five_star_book())