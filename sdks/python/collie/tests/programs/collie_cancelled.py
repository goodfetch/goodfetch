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

Given the Scripted engine's home as its second argument, the host raises
each SIGINT itself, from a thread of its own, once the engine has read the
call, as Windows delivers Ctrl+C: on a thread the console starts, never the
blocked main thread, which notices the signal only when its wait returns.
It then records, too, how long each interrupt took to end its call.
"""

from __future__ import annotations

import json
import logging
import os
import signal
import sys
import threading
import time
import warnings
from datetime import timedelta
from pathlib import Path
from typing import TYPE_CHECKING

from collie_interrupted import Heard
from goodfetch_test_support import scripted_engine_client_methods

if TYPE_CHECKING:
    from collections.abc import Callable

READ_DEADLINE_SECONDS = 60.0
"""How long the interrupting thread waits for the engine to read a call."""

INTERRUPTED_CALLS = ("locator.click", "run.finish")
"""The calls the interrupting thread raises SIGINT in, once the engine read each."""


class Interrupter:
    """Raises SIGINT from a thread of its own once the engine read each call."""

    def __init__(self, home: Path) -> None:
        """Starts the thread, which reads the record of the engine under `home`."""
        self._home = home
        self.raised_at: list[float] = []
        threading.Thread(target=self._interrupt, daemon=True).start()

    def _interrupt(self) -> None:
        deadline = time.monotonic() + READ_DEADLINE_SECONDS
        for method in INTERRUPTED_CALLS:
            while method not in scripted_engine_client_methods(self._home):
                if time.monotonic() > deadline:
                    return
                time.sleep(0.01)
            self.raised_at.append(time.monotonic())
            signal.raise_signal(signal.SIGINT)


def interrupted(call: Callable[[], object], ended_at: list[float]) -> bool:
    """Whether Ctrl+C's `KeyboardInterrupt` ended `call` rather than its answer.

    When it did, `ended_at` gets the moment it reached the caller.
    """
    try:
        call()
    except KeyboardInterrupt:
        ended_at.append(time.monotonic())
        return True
    return False


def main(out: Path, home: Path | None) -> None:
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
    interrupter = None if home is None else Interrupter(home)
    ended_at: list[float] = []
    click = interrupted(page.locator("#buy").click, ended_at)
    finish = interrupted(lambda: run.finish(timeout=timedelta(seconds=60)), ended_at)
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
                "interrupt_seconds": None
                if interrupter is None
                else [
                    ended - raised
                    for raised, ended in zip(
                        interrupter.raised_at, ended_at, strict=True
                    )
                ],
            }
        ),
        "utf-8",
    )


if __name__ == "__main__":
    # Python turns SIGINT into `KeyboardInterrupt` only when SIGINT is not
    # ignored at startup, and a test runner started as a shell's background
    # job ignores it and passes that on: the host takes Ctrl+C's way back.
    signal.signal(signal.SIGINT, signal.default_int_handler)
    main(Path(sys.argv[1]), Path(sys.argv[2]) if len(sys.argv) > 2 else None)
