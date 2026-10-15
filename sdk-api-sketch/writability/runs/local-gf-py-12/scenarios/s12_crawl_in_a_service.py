from fastapi import FastAPI, HTTPException, Query
from pydantic import BaseModel
from goodfetch_foxhound import Crawl, Label, EnqueueAdded
from goodfetch_foxhound._connection import Foxhound
from typing import List, TypeVar, Generic
import asyncio

app = FastAPI()

# Define the response model
class Book(BaseModel):
    title: str
    price: str

crawl = Crawl()
book_label = crawl.label(name="book", payload=object)

@app.get('/books', response_model=List[Book])
async def get_books(category: str = Query(..., description='Category of books to crawl')):
    async def handle_book(ctx):
        title = await ctx.page.text(".product_main h1")
        price = await ctx.page.text(".price_color")
        ctx.push(ctx.run.datasets['books'], {'title': title, 'price': price})

    crawl.handle(book_label)(handle_book)
    seeds = [crawl.seed(url=f'https://books.toscrape.com/{category}', label=book_label)]
    foxhound = Foxhound()
    async with foxhound.start_crawl(crawl, seeds) as run:
        async for result in run:
            if isinstance(result, EnqueueAdded):
                continue
            elif isinstance(result, dict):
                return [Book(title=item['title'], price=item['price']) for item in result['books']]
            else:
                raise HTTPException(status_code=500, detail='Unexpected result type')

    raise HTTPException(status_code=404, detail='No books found')