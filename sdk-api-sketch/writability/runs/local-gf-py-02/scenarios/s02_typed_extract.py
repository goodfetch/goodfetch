from goodfetch_collie_client import AsyncPage, AsyncSession
from pydantic import BaseModel
from fastapi import FastAPI
from typing import Optional

class Book(BaseModel):
    title: str
    price: float
    in_stock: int
    star_rating: int
    description: str

class ExtractionResult(BaseModel):
    book: Optional[Book] = None
    error: Optional[str] = None

class TokenUsage(BaseModel):
    tokens_used: int
    cost: float

async def extract_book_data(page: AsyncPage) -> Book:
    title = await page.get_by_role("heading", level=1).inner_text()
    price_str = await page.locator("p.price_color").inner_text()
    price = float(price_str.replace("£", ""))
    in_stock_str = await page.locator("p.instock.availability").inner_text()
    in_stock = int(in_stock_str.split()[2])
    star_rating_str = await page.locator("p.star-rating").get_attribute("class")
    star_rating = int(star_rating_str.split()[-1][-1])
    description = await page.locator("#product_description").inner_text()
    return Book(title=title, price=price, in_stock=in_stock, star_rating=star_rating, description=description)

app = FastAPI()

@app.post("/extract_book")
async def extract_book(session: AsyncSession) -> ExtractionResult:
    page = await session.new_page()
    await page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html")
    try:
        book = await extract_book_data(page)
        token_usage = TokenUsage(tokens_used=100, cost=0.01)  # Example values
        return ExtractionResult(book=book)
    except Exception as e:
        return ExtractionResult(error=str(e))
    finally:
        await page.close()

