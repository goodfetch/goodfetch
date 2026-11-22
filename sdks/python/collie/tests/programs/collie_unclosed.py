"""A program that connects with the sync API, starts a Run and never closes.

Before it returns, it prints the names of the threads other than its main
one that would keep it alive, those that are no daemon, as a JSON list: the
private thread and everything it starts never does (ADR 0081).
"""

from __future__ import annotations

import json
import sys
import threading

from goodfetch_collie import Collie


def main() -> None:
    """Connects, starts a Run and returns without closing either."""
    collie = Collie.connect()
    collie.start()
    holding = [
        thread.name
        for thread in threading.enumerate()
        if not thread.daemon and thread is not threading.main_thread()
    ]
    sys.stdout.write(json.dumps(holding))


if __name__ == "__main__":
    main()
