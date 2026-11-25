"""Prints the title of the page its argument names, else the Proving ground's.

Run it with `uv run main.py https://example.com/`.
"""

import sys

from goodfetch_collie import Collie
from title import read_title

url = sys.argv[1] if len(sys.argv) > 1 else "https://proving-ground.goodfetch.dev/"
with Collie.connect() as collie:
    sys.stdout.write(f"{read_title(collie, url)}\n")
