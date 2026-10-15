"""Golden scenario 01, "Fill a dependent form and read the results" (Collie, Python)."""

# The sync client, as the leader's program uses Playwright's sync API.
from dataclasses import dataclass

from goodfetch_collie import Collie


@dataclass(frozen=True)
class Quote:
    """A quote the search shows."""

    text: str
    author: str
    tag: str


def search_quotes(author: str, tag: str) -> list[Quote]:
    """Every quote the search shows for `author` and `tag`: an empty list when none matches."""
    # The Page check judges every page: one without the search form (an error page, a changed site) fails the call
    # that reached it with `PageCheckFailedError`, so an empty list is never a broken page.
    with (
        Collie.connect() as collie,
        collie.start(
            settings={
                "session": {
                    "page_check": {
                        "expected_page": {"selector": "form[name='filterform']"}
                    }
                }
            }
        ) as run,
    ):
        page = run.new_page()
        page.goto("https://quotes.toscrape.com/search.aspx")
        # Choosing the author posts the form back, and the action waits for that navigation to commit.
        page.get_by_label("Author").select_option(author)
        # The guard waits for the reloaded list to offer the tag. Each failure is typed and names the call, its locator
        # and what it saw (here the options offered), with the timeout's setting.
        page.get_by_label("Tag").select_option(tag)
        page.get_by_role("button", name="Search").click()
        page.wait_for_load_state()  # the results' document has loaded before it is read

        # An empty list is an answer, not an error (G1).
        quotes = [
            Quote(
                text=row.locator(".content").text_content(),
                author=row.locator(".author").text_content(),
                tag=row.locator(".tag").text_content(),
            )
            for row in page.locator(".results .quote").all()
        ]
        if not quotes:
            # The site heads its results "Results" only above a match: that heading over no quote is a changed layout.
            page.get_by_role("heading", name="Results").wait_for(state="hidden")
        run.finish()  # closes the Run's Session and page; `with` closes the rest on every path
        return quotes


if __name__ == "__main__":
    found = search_quotes("Albert Einstein", "inspirational")
    if not found:
        print("No quote matches Albert Einstein and inspirational.")
    for quote in found:
        print(f"{quote.text} ({quote.author}, {quote.tag})")
