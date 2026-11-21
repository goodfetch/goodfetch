"""A guest program: it imports, connects, runs the tracer and closes (ADR 0078).

It records what the SDK changed in its process at each stage into the file
its first argument names, with each warning the host's filters let through
and the stage it came at: import, connect, or the Run after them. It runs
one of the three ways its second argument names: `asyncio`, `trio` or
`sync`. It writes nothing to stdout or stderr itself, so anything there is
the SDK's. TypeScript's
`tests/fixtures/guest.ts` is its twin.
"""

from __future__ import annotations

import atexit
import importlib
import inspect
import json
import logging
import os
import signal
import sys
import threading
import warnings
from collections.abc import Iterator, Mapping, MutableMapping
from pathlib import Path
from typing import TypeAlias

import anyio
from collie_tracer import start_and_finish

SIGNALS = ("SIGINT", "SIGTERM", "SIGHUP", "SIGQUIT", "SIGCHLD", "SIGPIPE", "SIGBREAK")
"""The signals whose handlers a guest may never change, where the system has them."""


PLUMBING = frozenset({__name__, "os", "_collections_abc"})
"""The modules a read passes through on its way here, never its reader."""


class RecordingEnviron(MutableMapping[str, str]):
    """`os.environ` that keeps each variable read, with the module that read it.

    The standard library and the SDK's dependencies read variables of their
    own at import, such as `HOME` for `sysconfig` or Pydantic's
    `PYDANTIC_DISABLE_PLUGINS`, as Node's module loader does in TypeScript's
    guest: the module that read each one tells the SDK's reads apart.
    """

    def __init__(self, environ: MutableMapping[str, str]) -> None:
        """Wraps the process's own map."""
        self.environ = environ
        """The process's own map, which every call reaches."""
        self.read: list[tuple[str, str]] = []
        """Each read's module and variable, in order.

        `get`, `in` and `getenv` read through here.
        """

    def __getitem__(self, key: str) -> str:
        """The variable's value, its read recorded."""
        frame = inspect.currentframe()
        while frame is not None and frame.f_globals.get("__name__") in PLUMBING:
            frame = frame.f_back
        reader = "" if frame is None else str(frame.f_globals.get("__name__"))
        self.read.append((reader, key))
        return self.environ[key]

    def read_by(self, prefix: str) -> list[str]:
        """Each variable a module named `prefix` or under it read, sorted."""
        return sorted(
            {
                key
                for reader, key in self.read
                if reader == prefix
                or reader.startswith(f"{prefix}.")
                or reader.startswith(f"{prefix}_")
            }
        )

    def __setitem__(self, key: str, value: str) -> None:
        """Sets the variable."""
        self.environ[key] = value

    def __delitem__(self, key: str) -> None:
        """Removes the variable."""
        del self.environ[key]

    def __iter__(self) -> Iterator[str]:
        """Each variable's name."""
        return iter(self.environ)

    def __len__(self) -> int:
        """How many variables there are."""
        return len(self.environ)

    def copy(self) -> dict[str, str]:
        """A copy, as `os.environ.copy()` gives."""
        return dict(self.environ)


def handler(value: object) -> str:
    """A signal handler's name and identity.

    asyncio's handler is a `functools.partial` bound to its running task, whose `repr`
    changes as the task runs, so its identity tells a handler apart.
    """
    identity = id(value)
    function: object = getattr(value, "func", value)
    name = getattr(function, "__qualname__", repr(function))
    return f"{name} at {identity:#x}"


def state() -> dict[str, object]:
    """What a guest may never change: the process's handlers, hooks and threads."""
    root = logging.getLogger()
    return {
        "signals": {
            name: handler(signal.getsignal(getattr(signal, name)))
            for name in SIGNALS
            if hasattr(signal, name)
        },
        # The count is the only view `atexit` gives of its callbacks.
        "atexit": atexit._ncallbacks(),  # pyright: ignore[reportPrivateUsage]
        "root_logger": [root.level, [repr(handler) for handler in root.handlers]],
        "hooks": [
            repr(hook)
            for hook in (sys.excepthook, threading.excepthook, sys.unraisablehook)
        ],
        "warnings_filters": [repr(entry) for entry in warnings.filters],
        "private_threads": [
            thread.name
            for thread in threading.enumerate()
            if thread.name == "goodfetch-collie"
        ],
    }


Stages: TypeAlias = tuple[dict[str, object], int, dict[str, object], str]
"""The state before connecting, how many warnings were heard once connected, the
state after closing, and the Run's lifecycle.

Both states are read inside the event loop: asyncio and Trio install a
SIGINT handler of their own while their loop runs.
"""


async def run_async(
    env: Mapping[str, str], heard: list[warnings.WarningMessage]
) -> Stages:
    """Connects with the async API, runs the tracer and closes."""
    from goodfetch_collie import AsyncCollie

    before_connect = state()
    collie = await AsyncCollie.connect(env=env)
    connected = len(heard)
    traced = await start_and_finish(collie)
    return before_connect, connected, state(), traced.report.lifecycle


def run_sync(env: Mapping[str, str], heard: list[warnings.WarningMessage]) -> Stages:
    """Connects with the sync API, starts a Run, finishes it and closes."""
    from goodfetch_collie import Collie

    before_connect = state()
    with Collie.connect(env=env) as collie:
        connected = len(heard)
        report = collie.start().finish()
    return before_connect, connected, state(), report.lifecycle


def stage(index: int, imported: int, connected: int) -> str:
    """The stage of the warning heard `index`th."""
    if index < imported:
        return "import"
    return "connect" if index < connected else "run"


def main(out: Path, way: str) -> None:
    """Imports, connects and closes `way`, recording each stage into `out`."""
    env = dict(os.environ)
    recording = RecordingEnviron(os.environ)
    # The swap records reads, and clears nothing: the map behind it stays the same.
    os.__dict__["environ"] = recording
    # The host's channel (ADR 0078), heard before the first stage, so each
    # stage's filters are the same: each warning the host's filters let
    # through is recorded where they would have shown it on stderr.
    with warnings.catch_warnings(record=True) as heard:
        before_import = state()
        importlib.import_module("goodfetch_collie")

        after_import = state()
        imported = len(heard)
        os.__dict__["environ"] = recording.environ
        before_connect, connected, after_close, lifecycle = (
            run_sync(env, heard)
            if way == "sync"
            else anyio.run(run_async, env, heard, backend=way)
        )
    out.write_text(
        json.dumps(
            {
                "before_import": before_import,
                "after_import": after_import,
                "before_connect": before_connect,
                "after_close": after_close,
                "read_at_import": recording.read_by("goodfetch"),
                "every_read_at_import": sorted(
                    {f"{reader}: {key}" for reader, key in recording.read}
                ),
                "lifecycle": lifecycle,
                "warnings": [
                    {
                        "stage": stage(index, imported, connected),
                        "category": warning.category.__name__,
                        "message": str(warning.message),
                    }
                    for index, warning in enumerate(heard)
                ],
            }
        ),
        "utf-8",
    )


if __name__ == "__main__":
    main(Path(sys.argv[1]), sys.argv[2])
