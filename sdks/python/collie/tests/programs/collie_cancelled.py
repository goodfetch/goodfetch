"""A host program that Ctrl+C interrupts in the middle of its calls (ADRs 0080, 0081).

It connects with the sync API, starts a Run, opens a page and clicks
`#buy`, then finishes the Run with a timeout of 60 s, then of 2 ms, then
with none. The test sends it SIGINT once the engine has read the click, and
again once the engine has read the first finish: each `KeyboardInterrupt`
cancels the call it interrupts, which sends `$/cancel` naming it, and
propagates unchanged. The second finish's own timeout ends it first, with
`deadline_exceeded`; no Ctrl+C is armed beside it, since only the test
sends the signal, so the async ways alone arm a later cancellation there. It
then writes into the file its first argument names
how each call ended and what its `goodfetch` loggers heard at warning, and
exits 0. It writes nothing to stdout or stderr itself, so anything there is
the SDK's. `test_cancellation.py` plays the same script with the async API.
"""

from __future__ import annotations

import json
import logging
import os
import sys
import warnings
from datetime import timedelta
from pathlib import Path
from typing import TYPE_CHECKING

from collie_interrupted import Heard

if TYPE_CHECKING:
    from collections.abc import Callable


def interrupted(call: Callable[[], object]) -> bool:
    """Whether Ctrl+C's `KeyboardInterrupt` ended `call` rather than its answer."""
    try:
        call()
    except KeyboardInterrupt:
        return True
    return False


def main(out: Path) -> None:
    """Plays the calls, two of which Ctrl+C interrupts, and records them into `out`."""
    from goodfetch_collie import Collie, GoodfetchError

    heard = Heard()
    logging.getLogger("goodfetch").addHandler(heard)
    # The catalog's marker warnings (ADR 0030) reach the host's own warning
    # channel, which this host silences, so stderr holds only the SDK's output.
    warnings.simplefilter("ignore")
    collie = Collie.connect(env=dict(os.environ))
    run = collie.start()
    page = run.new_page()
    click = interrupted(page.locator("#buy").click)
    finish = interrupted(lambda: run.finish(timeout=timedelta(seconds=60)))
    timed_out: str | None = None
    try:
        run.finish(timeout=timedelta(milliseconds=2))
    except GoodfetchError as error:
        timed_out = error.code
    report = run.finish()
    collie.close()
    out.write_text(
        json.dumps(
            {
                "click_interrupted": click,
                "finish_interrupted": finish,
                "timed_out": timed_out,
                "lifecycle": report.lifecycle,
                "warnings": heard.messages,
            }
        ),
        "utf-8",
    )


if __name__ == "__main__":
    main(Path(sys.argv[1]))
