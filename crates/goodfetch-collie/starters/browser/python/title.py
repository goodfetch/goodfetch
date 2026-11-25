"""What the program does, apart from how it is started, so that its test runs it."""

from goodfetch_collie import Collie


def read_title(collie: Collie, url: str) -> str:
    """Opens `url` in a browser page of a new Run and answers the page's title."""
    run = collie.start()
    page = run.new_page()
    page.goto(url)
    title = page.evaluate("() => document.title")
    run.finish()
    return str(title)
