"""Prints the title of the page its argument names, else the Proving ground's.

Run it with `uv run main.py https://example.com/`.
"""

import sys

from goodfetch_collie import Collie


def read_title(collie: Collie, url: str) -> str:
    """Opens `url` in a browser page of a new Run and answers the page's title."""
    run = collie.start()
    page = run.new_page()
    page.goto(url)
    title = page.evaluate("() => document.title")
    run.finish()
    return str(title)


# Started as the program, not imported by its test.
if __name__ == "__main__":
    url = sys.argv[1] if len(sys.argv) > 1 else "https://proving-ground.goodfetch.dev/"
    with Collie.connect() as collie:
        sys.stdout.write(f"{read_title(collie, url)}\n")
