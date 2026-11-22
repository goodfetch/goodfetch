"""A program where `opentelemetry-api` does not import (ADR 0132).

A finder first on `sys.meta_path` refuses every `opentelemetry` module, as
an interpreter without the package does, and records each import tried.
The program then imports the SDK and makes one call through the sync API.
Once the call has passed, it prints, as one JSON object, the imports the
SDK's import tried and the OpenTelemetry modules loaded at the end. The
Scripted engine it reaches refuses a call with any `_meta`, so the call
passing says it sent none.
"""

from __future__ import annotations

import json
import sys
from importlib.abc import MetaPathFinder
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Sequence
    from importlib.machinery import ModuleSpec
    from types import ModuleType

PACKAGE = "opentelemetry"
"""The package the program has not installed."""


class Absent(MetaPathFinder):
    """Refuses each module of `PACKAGE`, and records the names asked."""

    def __init__(self) -> None:
        """No import tried yet."""
        self.tried: list[str] = []

    def find_spec(
        self,
        fullname: str,
        path: Sequence[str] | None,
        target: ModuleType | None = None,
    ) -> ModuleSpec | None:
        """Refuses `PACKAGE`'s modules, as a missing package is; leaves the rest."""
        del path, target
        if fullname.partition(".")[0] != PACKAGE:
            return None
        self.tried.append(fullname)
        message = f"No module named {fullname!r}"
        raise ModuleNotFoundError(message, name=fullname)


def main() -> None:
    """Imports the SDK where `PACKAGE` is absent, then makes one call."""
    absent = Absent()
    sys.meta_path.insert(0, absent)
    # Imported only once `PACKAGE` is absent.
    from goodfetch_collie import Collie

    tried_on_import = list(absent.tried)
    with Collie.connect() as collie:
        collie.explain_settings()
    loaded = sorted(name for name in sys.modules if name.partition(".")[0] == PACKAGE)
    sys.stdout.write(json.dumps({"tried_on_import": tried_on_import, "loaded": loaded}))


if __name__ == "__main__":
    main()
