```python
# s10_offline_crawl_tests.py

import asyncio
import unittest
from unittest.mock import patch, AsyncMock

from goodfetch_foxhound import AsyncCrawlContext, seed
from goodfetch_collie_client import Dataset, AsyncFoxhound
from scenarios.s05_two_page_crawl import crawl, books, home, category, book, RATINGS
from scenarios.s08_ai_in_crawl_handler import PLAN, on_book, main


class TestBookPageHandler(unittest.IsolatedAsyncioTestCase):
    async def test_book_page_handler(self):
        ctx = AsyncCrawlContext()
        ctx.page = AsyncMock()
        ctx.page.locator = AsyncMock()
        ctx.page.text_content = AsyncMock(return_value="""
            <h1>Sample Book</h1>
            <p class=\"price\">£10.99</p>
            <p class=\"instock availability\">(7 available)</p>
            <p class=\"star-rating Two\">&nbsp;</p>
        """)
        ctx.page.url = "https://books.toscrape.com/sample-book"
        ctx.request = AsyncMock()
        ctx.request.payload = AsyncMock()
        ctx.request.payload.category = "Sample Category"
        ctx.skip = AsyncMock()
        ctx.push = AsyncMock()

        await on_book(ctx)

        ctx.skip.assert_not_called()
        ctx.push.assert_called_once()
        ctx.push.assert_called_with(
            books,
            Dataset(
                title="Sample Book",
                price=Decimal("10.99"),
                in_stock=7,
                rating=2,
                url="https://books.toscrape.com/sample-book",
                category="Sample Category"
            )
        )


class TestOfflineCrawl(unittest.IsolatedAsyncioTestCase):
    async def test_offline_crawl(self):
        with patch("goodfetch_collie_client.AsyncFoxhound.connect") as mock_connect:
            mock_foxhound = AsyncMock()
            mock_run = AsyncMock()
            mock_run.export = AsyncMock(return_value=AsyncMock(records=100))
            mock_foxhound.start_crawl = AsyncMock(return_value=mock_run)
            mock_connect.return_value = mock_foxhound

            await main()

            mock_connect.assert_called_once()
            mock_foxhound.start_crawl.assert_called_once()
            mock_run.export.assert_called_once()
            mock_run.export.assert_called_with(books, format="json")


class TestAIInCrawlHandler(unittest.IsolatedAsyncioTestCase):
    async def test_ai_in_crawl_handler(self):
        with patch("goodfetch_collie_client.AsyncFoxhound.connect") as mock_connect:
            mock_foxhound = AsyncMock()
            mock_run = AsyncMock()
            mock_foxhound.start_crawl = AsyncMock(return_value=mock_run)
            mock_connect.return_value = mock_foxhound

            await main()

            mock_connect.assert_called_once()
            mock_foxhound.start_crawl.assert_called_once()


if __name__ == "__main__":
    unittest.main()
```