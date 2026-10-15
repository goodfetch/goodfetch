"""Golden scenario 01, "Fill a dependent form and read the results": the leader, Playwright 1.64.0 (Python)."""

# The sync API, which Playwright's Python docs show first in every example.
from dataclasses import dataclass

from playwright.sync_api import sync_playwright


@dataclass(frozen=True)
class Quote:
    text: str
    author: str
    tag: str


def search_quotes(author: str, tag: str) -> list[Quote]:
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page()
        page.goto("https://quotes.toscrape.com/search.aspx")
        # Choosing the author posts the form back; select_option on the tag waits for its option to appear.
        page.get_by_label("Author").select_option(author)
        page.get_by_label("Tag").select_option(tag)
        with page.expect_response("**/filter.aspx") as response_info:
            page.get_by_role("button", name="Search").click()
        response = response_info.value
        if not response.ok:
            raise RuntimeError(f"Search: filter.aspx answered {response.status}")
        page.wait_for_load_state()

        quotes = [
            Quote(
                text=row.locator(".content").inner_text(),
                author=row.locator(".author").inner_text(),
                tag=row.locator(".tag").inner_text(),
            )
            for row in page.locator(".results .quote").all()
        ]
        if not quotes:
            # The site heads its results "Results" only above a match: that heading over no quote is a changed layout.
            page.get_by_role("heading", name="Results").wait_for(state="hidden")
        browser.close()
        return quotes


if __name__ == "__main__":
    found = search_quotes("Albert Einstein", "inspirational")
    if not found:
        print("No quote matches Albert Einstein and inspirational.")
    for quote in found:
        print(f"{quote.text} ({quote.author}, {quote.tag})")
