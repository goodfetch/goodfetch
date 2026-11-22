"""Runs first in each Python program a test starts under `--mutation-children`.

Its folder is first on the program's `PYTHONPATH` (`mutation_children`), so
Python imports it at startup. It imports mutmut's trampoline, which every
mutated module imports, before the program's own code: a program that
compares the modules loaded before and after its import of the SDK sees the
same as with the sources. While mutmut learns which tests reach each
function (`MUTANT_UNDER_TEST=stats`), each function the program reaches is
written once, to a file named after the process in the hits folder, at
once: a program killed by a signal has still written its own.

mutmut calls each mutated function through frames of its own, which would
move a warning's `stacklevel` inside mutmut: the program's `warnings.warn`
does not count them, so a warning still shows its caller's line.

A Python without mutmut, such as the isolated one a build backend runs in,
runs no mutated code, and this does nothing there.
"""

import importlib.util
import inspect
import os
import warnings
from pathlib import Path
from types import FrameType


def _skip_trampoline_frames(trampoline: str) -> None:
    """Makes `warnings.warn` not count the frames of mutmut's `trampoline` file.

    Python's own `skip_file_prefixes` does not skip frames between the caller
    and its `stacklevel` (Python 3.12), so the level is counted here, frame by
    frame as Python counts it: past its import machinery's frames.
    """
    warn = warnings.warn

    def warn_past_trampoline(
        message: str | Warning,
        category: type[Warning] | None = None,
        stacklevel: int = 1,
        source: object = None,
        *,
        skip_file_prefixes: tuple[str, ...] = (),
    ) -> None:
        here = inspect.currentframe()
        frame = None if here is None else here.f_back
        # This frame and the caller's are Python's first two levels.
        level = 2
        wanted = stacklevel - 1
        while wanted > 0 and frame is not None:
            frame = _next_frame(frame)
            level += 1
            if frame is None or frame.f_code.co_filename != trampoline:
                wanted -= 1
        level += wanted
        if skip_file_prefixes:
            warn(
                message, category, level, source, skip_file_prefixes=skip_file_prefixes
            )
        else:
            warn(message, category, level, source)

    warnings.warn = warn_past_trampoline


def _next_frame(frame: FrameType) -> FrameType | None:
    """The frame `stacklevel` counts after `frame`: past the import machinery's."""
    caller = frame.f_back
    while caller is not None and _is_import_machinery(caller.f_code.co_filename):
        caller = caller.f_back
    return caller


def _is_import_machinery(filename: str) -> bool:
    """Whether Python's `warnings` skips `filename`'s frames, as its own."""
    return "importlib" in filename and "_bootstrap" in filename


def _record_hits(folder: Path) -> None:
    """Writes each function the mutated copy reaches once, to `folder`."""
    import mutmut.mutation.trampoline as trampoline

    seen: set[str] = set()

    def record(name: str, caller: str | None = None) -> None:
        del caller
        if name in seen:
            return
        seen.add(name)
        # A forked child writes to a file of its own.
        with (folder / str(os.getpid())).open("a", encoding="utf-8") as file:
            file.write(f"{name}\n")

    # The trampoline calls the name it imported from `mutmut.stats`, which it
    # does not export: its own globals hold it.
    trampoline.__dict__["record_trampoline_hit"] = record


if importlib.util.find_spec("mutmut") is not None:
    import mutmut.mutation.trampoline

    _skip_trampoline_frames(mutmut.mutation.trampoline.__file__)
    _hits = os.environ.get("MUTATION_CHILD_HITS")
    if _hits is not None and os.environ.get("MUTANT_UNDER_TEST") == "stats":
        _record_hits(Path(_hits))
